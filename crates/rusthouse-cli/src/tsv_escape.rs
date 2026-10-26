use std::io;

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
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
            b'0' => output.push(0),
            b'x' => {
                let high = *field
                    .get(index + 1)
                    .ok_or_else(|| invalid("short TSV hex escape"))?;
                let low = *field
                    .get(index + 2)
                    .ok_or_else(|| invalid("short TSV hex escape"))?;
                let high = hex_digit(high).ok_or_else(|| invalid("invalid TSV hex escape"))?;
                let low = hex_digit(low).ok_or_else(|| invalid("invalid TSV hex escape"))?;
                output.push((high << 4) | low);
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
    fn decodes_control_and_hex_escapes() {
        assert_eq!(
            decode_tsv_field(br"a\t\n\r\0\\\x41").unwrap(),
            b"a\t\n\r\0\\A"
        );
    }

    #[test]
    fn rejects_invalid_escapes() {
        for input in [br"\x0".as_slice(), br"\q".as_slice(), br"\".as_slice()] {
            assert!(decode_tsv_field(input).is_err());
        }
    }
}
