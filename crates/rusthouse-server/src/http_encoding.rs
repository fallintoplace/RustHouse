use super::*;

const MAX_ENCODED_QUERY_BYTES: usize = MAX_QUERY_BYTES * 3;

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
        let ch = bytes[index];
        if ch == b'%' {
            decoded.push(decode_percent_triplet(bytes, index)?);
            index += 3;
        } else {
            decoded.push(if ch == b'+' { b' ' } else { ch });
            index += 1;
        }
        if decoded.len() > MAX_QUERY_BYTES {
            return Err("SQL statement exceeds 4096 bytes");
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
            let mut quoted = [0u8; 2050];
            quoted[0] = b'"';
            quoted[1..bytes.len() + 1].copy_from_slice(bytes);
            quoted[bytes.len() + 1] = b'"';
            return writer.write_all(&quoted[..bytes.len() + 2]);
        }
        if bytes.len() <= 4094 {
            let mut quoted = [0u8; 4096];
            quoted[0] = b'"';
            quoted[1..bytes.len() + 1].copy_from_slice(bytes);
            quoted[bytes.len() + 1] = b'"';
            return writer.write_all(&quoted[..bytes.len() + 2]);
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
    for &byte in bytes {
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
