use std::io;

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn decode_hex_byte(field: &[u8], index: usize) -> io::Result<u8> {
    let digits = field
        .get(index..index + 2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "short TSV hex escape"))?;
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid TSV hex escape");
    let high = hex_digit(digits[0]).ok_or_else(invalid)?;
    let low = hex_digit(digits[1]).ok_or_else(invalid)?;
    Ok((high << 4) | low)
}

pub(crate) fn decode_tsv_field(field: &[u8]) -> io::Result<Vec<u8>> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
    let mut output = Vec::with_capacity(field.len());
    let mut index = 0;
    while index < field.len() {
        if field[index] != b'\\' {
            output.push(field[index]);
            index += 1;
            continue;
        }
        index += 1;
        let escaped = *field
            .get(index)
            .ok_or_else(|| invalid("incomplete TSV escape"))?;
        match escaped {
            b'\\' => output.push(b'\\'),
            b't' => output.push(b'\t'),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b'b' => output.push(8),
            b'f' => output.push(12),
            b'v' => output.push(11),
            b'0' => output.push(0),
            b'x' => {
                output.push(decode_hex_byte(field, index + 1)?);
                index += 2;
            }
            _ => return Err(invalid("unknown TSV escape")),
        }
        index += 1;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hex_pairs_and_keeps_error_categories() {
        assert_eq!(decode_hex_byte(b"aF", 0).unwrap(), 0xaf);
        assert_eq!(
            decode_hex_byte(b"0", 0).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            decode_hex_byte(b"g0", 0).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn decodes_control_and_hex_escapes() {
        assert_eq!(
            decode_tsv_field(br"a\t\n\r\0\\\x41").unwrap(),
            b"a\t\n\r\0\\A"
        );
    }

    #[test]
    fn decodes_additional_control_escapes() {
        assert_eq!(decode_tsv_field(br"\b\f\v").unwrap(), b"\x08\x0c\x0b");
    }

    #[test]
    fn rejects_invalid_escapes() {
        for input in [br"\x0".as_slice(), br"\q".as_slice(), br"\".as_slice()] {
            assert!(decode_tsv_field(input).is_err());
        }
    }
}
