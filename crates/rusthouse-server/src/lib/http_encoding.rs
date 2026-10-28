use super::*;

pub(super) fn hex_digit(value: u8) -> Option<u8> {
    (value as char).to_digit(16).map(|digit| digit as u8)
}

pub(super) fn decode_url(value: &str) -> Result<String, &'static str> {
    let bytes = value.as_bytes();
    if !bytes.contains(&b'%') && !bytes.contains(&b'+') {
        if bytes.len() > MAX_QUERY_BYTES {
            return Err("SQL statement exceeds 4096 bytes");
        }
        return Ok(value.to_owned());
    }
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index];
        if ch == b'%' {
            if index + 2 >= bytes.len() {
                return Err("invalid percent encoding");
            }
            let hi = hex_digit(bytes[index + 1]).ok_or("invalid percent encoding")?;
            let lo = hex_digit(bytes[index + 2]).ok_or("invalid percent encoding")?;
            decoded.push((hi << 4) | lo);
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
        writer.write_all(b"\"")?;
        writer.write_all(value.as_bytes())?;
        return writer.write_all(b"\"");
    }
    writer.write_all(b"\"")?;
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        let escape: Option<&[u8]> = match ch {
            '"' => Some(b"\\\""),
            '\\' => Some(b"\\\\"),
            '\n' => Some(b"\\n"),
            '\r' => Some(b"\\r"),
            '\t' => Some(b"\\t"),
            '\u{08}' => Some(b"\\b"),
            '\u{0c}' => Some(b"\\f"),
            ch if ch <= '\u{1f}' => {
                if start < index {
                    writer.write_all(&value.as_bytes()[start..index])?;
                }
                write!(writer, "\\u{:04x}", ch as u32)?;
                start = index + ch.len_utf8();
                continue;
            }
            _ => None,
        };
        if let Some(escape) = escape {
            if start < index {
                writer.write_all(&value.as_bytes()[start..index])?;
            }
            writer.write_all(escape)?;
            start = index + ch.len_utf8();
        }
    }
    if start < value.len() {
        writer.write_all(&value.as_bytes()[start..])?;
    }
    writer.write_all(b"\"")
}
