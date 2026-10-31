use super::*;

const MAX_ENCODED_QUERY_BYTES: usize = MAX_QUERY_BYTES * 3;

fn write_plain_json_string<const N: usize, W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    let mut quoted = [0u8; N];
    quoted[0] = b'"';
    quoted[1..bytes.len() + 1].copy_from_slice(bytes);
    quoted[bytes.len() + 1] = b'"';
    writer.write_all(&quoted[..bytes.len() + 2])
}

pub(super) fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn decode_percent_triplet(bytes: &[u8], index: usize) -> Result<u8, &'static str> {
    let digits = bytes
        .get(index + 1..index + 3)
        .ok_or("invalid percent encoding")?;
    let high = hex_digit(digits[0]).ok_or("invalid percent encoding")?;
    let low = hex_digit(digits[1]).ok_or("invalid percent encoding")?;
    Ok((high << 4) | low)
}

pub(super) fn decode_url(value: &str) -> Result<String, &'static str> {
    let bytes = value.as_bytes();
    if bytes.len() > MAX_ENCODED_QUERY_BYTES {
        return Err("URL-encoded SQL statement exceeds 12288 bytes");
    }
    if !bytes.contains(&b'%') {
        if bytes.len() > MAX_QUERY_BYTES {
            return Err("SQL statement exceeds 4096 bytes");
        }
        return Ok(if bytes.contains(&b'+') {
            value.replace('+', " ")
        } else {
            value.to_owned()
        });
    }
    // Percent escapes can triple the encoded length. Bound allocation by
    // the maximum decoded query size instead of the wire-size buffer.
    let mut decoded = Vec::with_capacity(bytes.len().min(MAX_QUERY_BYTES));
    let mut index = 0;
    while index < bytes.len() {
        // Copy ordinary UTF-8 spans in bulk. Only percent escapes and '+'
        // need individual decoding, even for long URL-encoded SQL queries.
        let span = bytes[index..]
            .iter()
            .position(|&byte| byte == b'%' || byte == b'+')
            .unwrap_or(bytes.len() - index);
        if span > MAX_QUERY_BYTES - decoded.len() {
            return Err("SQL statement exceeds 4096 bytes");
        }
        decoded.extend_from_slice(&bytes[index..index + span]);
        index += span;
        if index == bytes.len() {
            break;
        }
        if decoded.len() == MAX_QUERY_BYTES {
            return Err("SQL statement exceeds 4096 bytes");
        }
        if bytes[index] == b'%' {
            decoded.push(decode_percent_triplet(bytes, index)?);
            index += 3;
        } else {
            decoded.push(b' ');
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| "SQL query must be valid UTF-8")
}

pub(super) fn write_json_string<W: Write>(writer: &mut W, value: &str) -> io::Result<()> {
    let bytes = value.as_bytes();
    // Plain UTF-8 fields are copied without transformation.
    if !bytes.iter().any(|&byte| byte < 0x20 || byte == b'"' || byte == b'\\') {
        if bytes.len() <= 2048 {
            // Quote-free fields frequently exceed 256 bytes. Avoid three
            // buffered writes when they still fit in a small stack array.
            return write_plain_json_string::<2050, _>(writer, bytes);
        }
        if bytes.len() <= 4094 {
            return write_plain_json_string::<4096, _>(writer, bytes);
        }
        writer.write_all(b"\"")?;
        writer.write_all(bytes)?;
        return writer.write_all(b"\"");
    }

    // Batch frequent JSON escapes into stack chunks. Inspect bytes because
    // every JSON-special/control character is ASCII; other UTF-8 bytes pass
    // through unchanged.
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut buffer = [0u8; 4096];
    buffer[0] = b'"';
    let mut used = 1;
    let mut remaining = bytes;
    while !remaining.is_empty() {
        // JSON-special bytes are ASCII. Copy ordinary UTF-8 spans in bulk
        // instead of branching and copying each byte independently.
        let span = remaining
            .iter()
            .position(|&byte| byte < 0x20 || byte == b'"' || byte == b'\\')
            .unwrap_or(remaining.len());
        if span != 0 {
            let plain = &remaining[..span];
            if plain.len() >= buffer.len() {
                writer.write_all(&buffer[..used])?;
                writer.write_all(plain)?;
                used = 0;
            } else {
                if buffer.len() - used < plain.len() {
                    writer.write_all(&buffer[..used])?;
                    used = 0;
                }
                buffer[used..used + plain.len()].copy_from_slice(plain);
                used += plain.len();
            }
            remaining = &remaining[span..];
        }
        if remaining.is_empty() {
            break;
        }
        let byte = remaining[0];
        remaining = &remaining[1..];
        let mut unicode_escape = [0u8; 6];
        let encoded: &[u8] = match byte {
            b'"' => br#"\""#,
            b'\\' => br"\\",
            b'\n' => br"\n",
            b'\r' => br"\r",
            b'\t' => br"\t",
            8 => br"\b",
            12 => br"\f",
            0..=31 => {
                unicode_escape = [
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[(byte >> 4) as usize],
                    HEX[(byte & 0xf) as usize],
                ];
                &unicode_escape
            }
            _ => std::slice::from_ref(&byte),
        };
        if buffer.len() - used < encoded.len() {
            writer.write_all(&buffer[..used])?;
            used = 0;
        }
        buffer[used..used + encoded.len()].copy_from_slice(encoded);
        used += encoded.len();
    }
    if used == buffer.len() {
        writer.write_all(&buffer)?;
        used = 0;
    }
    buffer[used] = b'"';
    writer.write_all(&buffer[..used + 1])
}

#[cfg(test)]
mod percent_decoding_allocation_tests {
    use super::*;

    #[test]
    fn decodes_maximum_percent_encoded_sql_without_overallocating() {
        let encoded = "%41".repeat(MAX_QUERY_BYTES);
        let value = decode_url(&encoded).unwrap();
        assert_eq!(value.len(), MAX_QUERY_BYTES);
        assert!(value.bytes().all(|byte| byte == b'A'));
        assert_eq!(decode_url("SELECT+1%2B2").unwrap(), "SELECT 1+2");
        assert!(decode_url("%ZZ").is_err());
    }
}

#[cfg(test)]
#[path = "http_encoding/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "http_encoding/batched_json_escape_tests.rs"]
mod batched_json_escape_tests;

#[cfg(test)]
mod medium_json_string_write_tests {
    use super::*;

    struct Counted {
        calls: usize,
        bytes: Vec<u8>,
    }
    impl Write for Counted {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn plain_json_strings_up_to_two_kibibytes_need_one_write() {
        for length in [257, 512, 2048] {
            let value = "q".repeat(length);
            let mut out = Counted { calls: 0, bytes: Vec::new() };
            write_json_string(&mut out, &value).unwrap();
            assert_eq!(out.calls, 1);
            assert_eq!(out.bytes.len(), length + 2);
            assert_eq!(out.bytes[0], b'"');
            assert_eq!(out.bytes[length + 1], b'"');
        }
    }

    #[test]
    fn json_special_characters_still_use_correct_escaping() {
        let mut out = Vec::new();
        write_json_string(&mut out, "start\n\"quoted\"").unwrap();
        assert_eq!(out, b"\"start\\n\\\"quoted\\\"\"");
    }
}

#[cfg(test)]
mod sparse_percent_span_tests {
    use super::*;

    #[test]
    fn copies_long_plain_runs_around_escapes_and_plus_signs() {
        let first = "x".repeat(2048);
        let second = "y".repeat(2044);
        let encoded = format!("{first}%20+{second}");
        let decoded = decode_url(&encoded).unwrap();
        assert_eq!(decoded, format!("{first}  {second}"));
        assert_eq!(decoded.len(), 4094);
    }

    #[test]
    fn respects_exact_decoded_limit_and_utf8_validation() {
        let value = "v".repeat(MAX_QUERY_BYTES - 1);
        assert_eq!(decode_url(&format!("{value}%20")).unwrap().len(), MAX_QUERY_BYTES);
        assert!(decode_url(&format!("{value}%20x")).is_err());
        assert_eq!(decode_url("caf%C3%A9+bar").unwrap(), "café bar");
        assert!(decode_url("%FF").is_err());
        assert!(decode_url("%").is_err());
        assert!(decode_url("%G1").is_err());
    }

    #[test]
    fn handles_consecutive_escapes_and_empty_spans() {
        assert_eq!(decode_url("%41%42++%43").unwrap(), "AB  C");
        assert_eq!(decode_url("%00").unwrap().as_bytes(), &[0]);
    }
}

#[cfg(test)]
mod sparse_json_escape_span_tests {
    use super::*;

    #[test]
    fn long_plain_runs_around_escapes_preserve_utf8_and_control_bytes() {
        let left = "é".repeat(3000);
        let right = "猫".repeat(1800);
        let mut input = left.clone();
        input.push('\n');
        input.push('\u{0001}');
        input.push('\\');
        input.push('"');
        input.push_str(&right);
        input.push('\t');
        let mut expected = vec![b'"'];
        expected.extend_from_slice(left.as_bytes());
        expected.extend_from_slice(&[
            b'\\', b'n', b'\\', b'u', b'0', b'0', b'0', b'1',
            b'\\', b'\\', b'\\', b'"',
        ]);
        expected.extend_from_slice(right.as_bytes());
        expected.extend_from_slice(&[b'\\', b't', b'"']);
        let mut output = Vec::new();
        write_json_string(&mut output, &input).unwrap();
        assert_eq!(output, expected);
    }
}
