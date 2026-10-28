use super::*;

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
    if bytes.len() > MAX_QUERY_BYTES * 3 {
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
    let mut decoded = Vec::with_capacity(bytes.len());
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
    // UTF-8 bytes other than ASCII controls, quotes and backslashes are safe.
    if !value
        .as_bytes()
        .iter()
        .any(|byte| *byte < 0x20 || *byte == b'\"' || *byte == b'\\')
    {
        if value.len() <= 256 {
            let mut quoted = [0u8; 258];
            quoted[0] = b'"';
            quoted[1..value.len() + 1].copy_from_slice(value.as_bytes());
            quoted[value.len() + 1] = b'"';
            return writer.write_all(&quoted[..value.len() + 2]);
        }
        writer.write_all(b"\"")?;
        writer.write_all(value.as_bytes())?;
        return writer.write_all(b"\"");
    }
    let mut buffer = [0u8; 4096];
    let mut used = 1;
    buffer[0] = b'"';
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &byte in value.as_bytes() {
        let mut control = [b'\\', b'u', b'0', b'0', 0, 0];
        let escape: &[u8] = match byte {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            8 => b"\\b",
            12 => b"\\f",
            0..=31 => {
                control[4] = HEX[usize::from(byte >> 4)];
                control[5] = HEX[usize::from(byte & 0x0f)];
                &control
            }
            _ => std::slice::from_ref(&byte),
        };
        if used + escape.len() > buffer.len() {
            writer.write_all(&buffer[..used])?;
            used = 0;
        }
        buffer[used..used + escape.len()].copy_from_slice(escape);
        used += escape.len();
    }
    if used == buffer.len() {
        writer.write_all(&buffer[..used])?;
        used = 0;
    }
    buffer[used] = b'"';
    writer.write_all(&buffer[..used + 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_plain_json_fields_use_one_write() {
        struct CountingWriter {
            calls: usize,
            bytes: Vec<u8>,
        }

        impl Write for CountingWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut writer = CountingWriter {
            calls: 0,
            bytes: Vec::new(),
        };
        write_json_string(&mut writer, "hello").unwrap();
        assert_eq!(writer.calls, 1);
        assert_eq!(writer.bytes, br#""hello""#);
    }

    #[test]
    fn medium_json_string_uses_single_write() {
        struct Count {
            calls: usize,
            bytes: Vec<u8>,
        }
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let value = "é".repeat(100);
        let mut sink = Count {
            calls: 0,
            bytes: Vec::new(),
        };
        write_json_string(&mut sink, &value).unwrap();
        assert_eq!(sink.calls, 1);
        assert_eq!(sink.bytes, format!("\"{value}\"").as_bytes());
    }

    #[test]
    fn parses_only_ascii_hex_digits() {
        assert_eq!(hex_digit(b'0'), Some(0));
        assert_eq!(hex_digit(b'9'), Some(9));
        assert_eq!(hex_digit(b'a'), Some(10));
        assert_eq!(hex_digit(b'F'), Some(15));
        assert_eq!(hex_digit(b'G'), None);
        assert_eq!(hex_digit(0xff), None);
        assert_eq!(decode_url("%4f%4A").unwrap(), "OJ");
    }

    #[test]
    fn plus_only_queries_keep_utf8_without_percent_decoding() {
        assert_eq!(
            decode_url("SELECT+café+FROM+items").unwrap(),
            "SELECT café FROM items"
        );
        assert_eq!(decode_url("a+b%2Bc").unwrap(), "a b+c");
        assert!(decode_url(&"a+".repeat(MAX_QUERY_BYTES)).is_err());
    }

    #[test]
    fn bounds_encoded_query_size_before_allocation() {
        let maximum = "%41".repeat(MAX_QUERY_BYTES);
        assert_eq!(decode_url(&maximum).unwrap().len(), MAX_QUERY_BYTES);
        assert!(decode_url(&format!("{maximum}%41")).is_err());
        assert!(decode_url(&("a".repeat(MAX_QUERY_BYTES * 4))).is_err());
    }
    #[test]
    fn quote_heavy_json_uses_bounded_output_writes() {
        struct CountingWriter {
            calls: usize,
            output: Vec<u8>,
        }

        impl Write for CountingWriter {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.output.extend_from_slice(data);
                Ok(data.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let input = "\"".repeat(10_000);
        let mut output = CountingWriter {
            calls: 0,
            output: Vec::new(),
        };
        write_json_string(&mut output, &input).unwrap();
        assert_eq!(output.output.len(), 20_002);
        assert_eq!(&output.output[..3], b"\"\\\"");
        assert_eq!(output.output.last(), Some(&b'"'));
        assert!(output.calls <= 6);
        let mut control = Vec::new();
        write_json_string(&mut control, "a\n\t\u{0001}café").unwrap();
        assert_eq!(control, r#""a\n\t\u0001café""#.as_bytes());
    }


}
