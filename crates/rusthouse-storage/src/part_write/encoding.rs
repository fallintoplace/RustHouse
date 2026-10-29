use super::*;

pub(super) fn push_varuint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn push_uuid(value: &str, output: &mut Vec<u8>) -> Result<(), PartError> {
    let bytes = value.as_bytes();
    if bytes.len() != 36 || [8, 13, 18, 23].iter().any(|&index| bytes[index] != b'-') {
        return Err(invalid(format!("invalid UUID value: {value}")));
    }
    let mut canonical = [0u8; 16];
    let mut digit_index = 0usize;
    for (position, &byte) in bytes.iter().enumerate() {
        if matches!(position, 8 | 13 | 18 | 23) {
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return Err(invalid(format!("invalid UUID value: {value}"))),
        };
        let slot = &mut canonical[digit_index / 2];
        if digit_index % 2 == 0 {
            *slot = digit << 4;
        } else {
            *slot |= digit;
        }
        digit_index += 1;
    }
    // ClickHouse stores each UInt64 UUID half in little-endian order.
    for half in canonical.chunks_exact_mut(8) {
        half.reverse();
    }
    output.extend_from_slice(&canonical);
    Ok(())
}

pub(super) fn push_cell(
    kind: &str,
    value: Option<&str>,
    output: &mut Vec<u8>,
) -> Result<bool, PartError> {
    let (base, nullable) = base_type(kind);
    let is_null = value.is_none();
    if is_null && !nullable {
        return Err(invalid(format!("NULL is not permitted for {kind}")));
    }
    let value = value.unwrap_or(if base == "String" { "" } else { "0" });
    macro_rules! numeric {
        ($ty:ty) => {{
            let number = value
                .parse::<$ty>()
                .map_err(|_| invalid(format!("invalid {kind} value: {value}")))?;
            output.extend_from_slice(&number.to_le_bytes());
        }};
    }
    match base {
        "String" => {
            push_varuint(value.len() as u64, output);
            output.extend_from_slice(value.as_bytes());
        }
        "UUID" => push_uuid(value, output)?,
        "Date" => {
            let days = crate::parse_date(value)
                .or_else(|| value.parse::<u16>().ok())
                .ok_or_else(|| invalid(format!("invalid Date value: {value}")))?;
            output.extend_from_slice(&days.to_le_bytes());
        }
        "DateTime" => {
            let seconds = crate::parse_datetime(value)
                .or_else(|| value.parse::<u32>().ok())
                .ok_or_else(|| invalid(format!("invalid DateTime value: {value}")))?;
            output.extend_from_slice(&seconds.to_le_bytes());
        }
        kind if crate::datetime64_scale(kind).is_some() => {
            let precision = crate::datetime64_scale(kind).expect("validated DateTime64 kind");
            let ticks = crate::parse_datetime64(value, precision)
                .or_else(|| value.parse::<i64>().ok())
                .ok_or_else(|| invalid(format!("invalid DateTime64 value: {value}")))?;
            output.extend_from_slice(&ticks.to_le_bytes());
        }
        "UInt8" => numeric!(u8),
        "UInt16" => numeric!(u16),
        "UInt32" => numeric!(u32),
        "UInt64" => numeric!(u64),
        "UInt128" => numeric!(u128),
        "Int8" => numeric!(i8),
        "Int16" => numeric!(i16),
        "Int32" => numeric!(i32),
        "Int64" => numeric!(i64),
        "Int128" => numeric!(i128),
        "Float32" => {
            let number = value
                .parse::<f32>()
                .map_err(|_| invalid(format!("invalid {kind} value: {value}")))?;
            if !number.is_finite() {
                return Err(invalid("non-finite float values are unsupported"));
            }
            output.extend_from_slice(&number.to_le_bytes());
        }
        "Float64" => {
            let number = value
                .parse::<f64>()
                .map_err(|_| invalid(format!("invalid {kind} value: {value}")))?;
            if !number.is_finite() {
                return Err(invalid("non-finite float values are unsupported"));
            }
            output.extend_from_slice(&number.to_le_bytes());
        }
        _ => return Err(invalid(format!("unsupported writable column type: {kind}"))),
    }
    if output.len() > MAX_STREAM_BYTES {
        return Err(invalid("part column exceeds 8 MiB"));
    }
    Ok(is_null)
}

// Parses JSON-style string keys and String/numeric values. Map preserves duplicate keys.
fn skip_map_ws(input: &[u8], position: &mut usize) {
    while input.get(*position).is_some_and(u8::is_ascii_whitespace) {
        *position += 1;
    }
}

fn read_map_hex(input: &[u8], position: &mut usize) -> Result<u16, PartError> {
    let mut number = 0u16;
    for _ in 0..4 {
        let byte = *input
            .get(*position)
            .ok_or_else(|| invalid("incomplete Map Unicode escape"))?;
        let digit = match byte {
            b'0'..=b'9' => u16::from(byte - b'0'),
            b'a'..=b'f' => u16::from(byte - b'a' + 10),
            b'A'..=b'F' => u16::from(byte - b'A' + 10),
            _ => return Err(invalid("invalid Map Unicode escape")),
        };
        number = (number << 4) | digit;
        *position += 1;
    }
    Ok(number)
}

fn read_map_string<'a>(
    input: &'a str,
    position: &mut usize,
) -> Result<std::borrow::Cow<'a, str>, PartError> {
    let bytes = input.as_bytes();
    if bytes.get(*position) != Some(&b'"') {
        return Err(invalid("Map keys and String values require JSON quotes"));
    }
    *position += 1;
    let mut fragment = *position;
    let mut output = String::new();
    let mut had_escape = false;
    while let Some(&byte) = bytes.get(*position) {
        if byte == b'"' {
            if had_escape {
                output.push_str(&input[fragment..*position]);
                *position += 1;
                return Ok(std::borrow::Cow::Owned(output));
            }
            let plain = &input[fragment..*position];
            *position += 1;
            return Ok(std::borrow::Cow::Borrowed(plain));
        }
        if byte < 0x20 {
            return Err(invalid("unescaped control byte in Map string"));
        }
        if byte != b'\\' {
            *position += 1;
            continue;
        }
        had_escape = true;
        output.push_str(&input[fragment..*position]);
        *position += 1;
        let escaped = *bytes
            .get(*position)
            .ok_or_else(|| invalid("unterminated Map string escape"))?;
        *position += 1;
        match escaped {
            b'"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'/' => output.push('/'),
            b'b' => output.push('\u{0008}'),
            b'f' => output.push('\u{000C}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' => {
                let first = read_map_hex(bytes, position)?;
                let codepoint = if (0xD800..=0xDBFF).contains(&first) {
                    if bytes.get(*position..*position + 2) != Some(&b"\\u"[..]) {
                        return Err(invalid("Map Unicode high surrogate needs low surrogate"));
                    }
                    *position += 2;
                    let low = read_map_hex(bytes, position)?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return Err(invalid("invalid Map Unicode surrogate pair"));
                    }
                    0x10000 + (u32::from(first - 0xD800) << 10) + u32::from(low - 0xDC00)
                } else {
                    if (0xDC00..=0xDFFF).contains(&first) {
                        return Err(invalid("isolated Map Unicode low surrogate"));
                    }
                    u32::from(first)
                };
                output.push(
                    char::from_u32(codepoint)
                        .ok_or_else(|| invalid("invalid Map Unicode codepoint"))?,
                );
            }
            _ => return Err(invalid("invalid Map string escape")),
        }
        fragment = *position;
    }
    Err(invalid("unterminated Map string"))
}

/// Encode an unbucketed ClickHouse Map(String, T) as keys/values/size0 streams.
pub(super) fn push_string_key_map(
    value_type: &str,
    literal: Option<&str>,
    keys: &mut Vec<u8>,
    values: &mut Vec<u8>,
    sizes: &mut Vec<u8>,
) -> Result<(), PartError> {
    let literal = literal.ok_or_else(|| invalid("NULL is not permitted for Map"))?;
    let bytes = literal.as_bytes();
    let mut cursor = 0;
    skip_map_ws(bytes, &mut cursor);
    if bytes.get(cursor) != Some(&b'{') {
        return Err(invalid("Map requires a JSON-style object"));
    }
    cursor += 1;
    skip_map_ws(bytes, &mut cursor);
    let mut pairs = 0u64;
    if bytes.get(cursor) == Some(&b'}') {
        cursor += 1;
    } else {
        loop {
            let key = read_map_string(literal, &mut cursor)?;
            skip_map_ws(bytes, &mut cursor);
            if bytes.get(cursor) != Some(&b':') {
                return Err(invalid("expected ':' after Map key"));
            }
            cursor += 1;
            skip_map_ws(bytes, &mut cursor);
            let value: std::borrow::Cow<'_, str> = if value_type == "String" {
                read_map_string(literal, &mut cursor)?
            } else {
                let begin = cursor;
                while bytes
                    .get(cursor)
                    .is_some_and(|byte| !matches!(byte, b',' | b'}') && !byte.is_ascii_whitespace())
                {
                    cursor += 1;
                }
                if cursor == begin {
                    return Err(invalid("Map numeric value is missing"));
                }
                std::borrow::Cow::Borrowed(&literal[begin..cursor])
            };
            push_cell("String", Some(key.as_ref()), keys)?;
            push_cell(value_type, Some(value.as_ref()), values)?;
            pairs = pairs
                .checked_add(1)
                .ok_or_else(|| invalid("Map has too many pairs"))?;
            skip_map_ws(bytes, &mut cursor);
            match bytes.get(cursor) {
                Some(b',') => {
                    cursor += 1;
                    skip_map_ws(bytes, &mut cursor);
                }
                Some(b'}') => {
                    cursor += 1;
                    break;
                }
                _ => return Err(invalid("expected ',' or '}' after Map entry")),
            }
        }
    }
    skip_map_ws(bytes, &mut cursor);
    if cursor != bytes.len() {
        return Err(invalid("unexpected tokens after Map object"));
    }
    sizes.extend_from_slice(&pairs.to_le_bytes());
    Ok(())
}

pub(super) fn push_numeric_array(
    inner: &str,
    literal: Option<&str>,
    elements: &mut Vec<u8>,
    sizes: &mut Vec<u8>,
) -> Result<(), PartError> {
    let literal = literal.ok_or_else(|| invalid("NULL is not permitted for Array"))?;
    let body = literal
        .trim()
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| invalid("Array values require a bracketed list"))?;
    let mut count = 0u64;
    if !body.trim().is_empty() {
        for element in body.split(',') {
            let value = element.trim();
            if value.is_empty() {
                return Err(invalid("Array cannot contain an empty element"));
            }
            push_cell(inner, Some(value), elements)?;
            count = count
                .checked_add(1)
                .ok_or_else(|| invalid("Array has too many elements"))?;
        }
    }
    sizes.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

pub(super) fn checksum_hash(data: &[u8]) -> u128 {
    const HASH_BLOCK_BYTES: usize = 2048;
    let mut seed = U128::new(0, 0);
    let mut chunks = data.chunks_exact(HASH_BLOCK_BYTES);
    for chunk in chunks.by_ref() {
        seed = cityhash128_with_seed(chunk, seed);
    }
    if !chunks.remainder().is_empty() {
        seed = cityhash128_with_seed(chunks.remainder(), seed);
    }
    seed.into()
}

pub(super) fn encode_checksums(files: &[FileChecksum]) -> Vec<u8> {
    const HEADER: &[u8] = b"checksums format version: 3\n";
    // Account for names, checksums, and bounded varuint fields so metadata
    // encoding rarely needs to grow its buffer during part publication.
    let capacity = files.iter().fold(HEADER.len() + 10, |size, file| {
        size.saturating_add(file.name.len())
            .saturating_add(if file.uncompressed.is_some() { 64 } else { 40 })
    });
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(HEADER);
    push_varuint(files.len() as u64, &mut bytes);
    for file in files {
        push_varuint(file.name.len() as u64, &mut bytes);
        bytes.extend_from_slice(file.name.as_bytes());
        push_varuint(file.size, &mut bytes);
        bytes.extend_from_slice(&file.hash.to_le_bytes());
        match &file.uncompressed {
            Some(original) => {
                bytes.push(1);
                push_varuint(original.size, &mut bytes);
                bytes.extend_from_slice(&original.hash.to_le_bytes());
            }
            None => bytes.push(0),
        }
    }
    bytes
}

#[cfg(test)]
mod checksum_preallocation_tests {
    use super::*;

    #[test]
    fn checksum_preallocation_preserves_wire_bytes() {
        let file = FileChecksum {
            name: "a".into(),
            size: 1,
            hash: 0,
            uncompressed: None,
        };
        let mut expected = b"checksums format version: 3\n".to_vec();
        expected.extend_from_slice(&[1, 1, b'a', 1]);
        expected.extend_from_slice(&0u128.to_le_bytes());
        expected.push(0);
        assert_eq!(encode_checksums(&[file]), expected);
    }
}

#[cfg(test)]
mod uuid_writer_tests {
    use super::*;

    #[test]
    fn writes_clickhouse_uuid_byte_order_and_rejects_bad_literals() {
        let mut output = Vec::new();
        push_cell(
            "UUID",
            Some("61f0c404-5cb3-11e7-907b-a6006ad3dba0"),
            &mut output,
        )
        .unwrap();
        assert_eq!(
            output,
            [
                0xe7, 0x11, 0xb3, 0x5c, 0x04, 0xc4, 0xf0, 0x61, 0xa0, 0xdb, 0xd3, 0x6a, 0x00, 0xa6,
                0x7b, 0x90,
            ]
        );
        for bad in [
            "",
            "00000000-0000-0000-0000-00000000000z",
            "000000000000-0000-0000-000000000000",
            "00000000-0000-0000-0000-0000000000000",
        ] {
            assert!(
                push_cell("UUID", Some(bad), &mut Vec::new()).is_err(),
                "{bad}"
            );
        }
        assert!(push_cell("UUID", None, &mut Vec::new()).is_err());
    }
}

#[cfg(test)]
mod borrow_map_string_tests {
    use super::*;
    use std::borrow::Cow;

    #[test]
    fn plain_map_text_borrows_and_escaped_values_own() {
        let input = "\"café 東京\"";
        let mut position = 0;
        let value = read_map_string(input, &mut position).unwrap();
        assert!(matches!(value, Cow::Borrowed("café 東京")));
        assert_eq!(position, input.len());

        let input = "\"line\\nnext\"";
        let mut position = 0;
        let value = read_map_string(input, &mut position).unwrap();
        assert!(matches!(&value, Cow::Owned(_)));
        assert_eq!(value.as_ref(), "line\nnext");

        let input = "\"\"";
        let mut position = 0;
        assert!(matches!(
            read_map_string(input, &mut position).unwrap(),
            Cow::Borrowed("")
        ));
    }
}
