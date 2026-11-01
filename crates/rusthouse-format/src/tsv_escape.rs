use std::io;

fn invalid_escape(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn checked_hex_digit(value: u8) -> io::Result<u8> {
    hex_digit(value).ok_or_else(|| invalid_escape("invalid TSV hex escape"))
}

fn decode_hex_byte(field: &[u8], index: usize) -> io::Result<u8> {
    let digits = field
        .get(index..index + 2)
        .ok_or_else(|| invalid_escape("short TSV hex escape"))?;
    let high = checked_hex_digit(digits[0])?;
    let low = checked_hex_digit(digits[1])?;
    Ok((high << 4) | low)
}

fn decode_simple_tsv_escape(value: u8) -> Option<u8> {
    match value {
        b'\\' => Some(b'\\'),
        b't' => Some(b'\t'),
        b'n' => Some(b'\n'),
        b'r' => Some(b'\r'),
        b'b' => Some(8),
        b'f' => Some(12),
        b'v' => Some(11),
        b'0' => Some(0),
        _ => None,
    }
}

fn decode_escape(field: &[u8], index: &mut usize) -> io::Result<u8> {
    *index += 1;
    let escaped = *field
        .get(*index)
        .ok_or_else(|| invalid_escape("incomplete TSV escape"))?;
    if escaped == b'x' {
        let decoded = decode_hex_byte(field, *index + 1)?;
        *index += 3;
        Ok(decoded)
    } else {
        let decoded =
            decode_simple_tsv_escape(escaped).ok_or_else(|| invalid_escape("unknown TSV escape"))?;
        *index += 1;
        Ok(decoded)
    }
}

/// Check TSV escape syntax without allocating a decoded field.
/// Use this when forwarding peer rows without materializing their values.
pub fn validate_tsv_field(field: &[u8]) -> io::Result<()> {
    let mut cursor = 0;
    while let Some(relative) = field[cursor..].iter().position(|&byte| byte == b'\\') {
        cursor += relative;
        decode_escape(field, &mut cursor)?;
    }
    Ok(())
}

/// Decode an escaped TSV field into a reusable buffer.
/// The output is cleared before decoding. The caller can keep its capacity
/// across many rows to avoid a per-field allocation.
pub fn decode_tsv_field_into(field: &[u8], output: &mut Vec<u8>) -> io::Result<()> {
    output.clear();
    let Some(first_escape) = field.iter().position(|&byte| byte == b'\\') else {
        output.extend_from_slice(field);
        return Ok(());
    };
    output.reserve(field.len());
    let mut index = first_escape;
    let mut start = 0;
    loop {
        output.extend_from_slice(&field[start..index]);
        output.push(decode_escape(field, &mut index)?);
        start = index;
        let Some(next_escape_offset) = field[index..].iter().position(|&byte| byte == b'\\') else {
            break;
        };
        index += next_escape_offset;
    }
    output.extend_from_slice(&field[start..]);
    Ok(())
}

pub fn decode_tsv_field(field: &[u8]) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    decode_tsv_field_into(field, &mut output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_into_reuses_capacity_across_escaped_fields() {
        let mut buffer = Vec::with_capacity(4096);
        let original = buffer.as_ptr();
        for field in [
            br"caf\xC3\xA9".as_slice(),
            br"first\tsecond".as_slice(),
            br"plain".as_slice(),
            br"\n\r\0".as_slice(),
        ] {
            let owned = decode_tsv_field(field).unwrap();
            decode_tsv_field_into(field, &mut buffer).unwrap();
            assert_eq!(buffer, owned);
            assert_eq!(buffer.as_ptr(), original);
        }
        assert!(decode_tsv_field_into(br"bad\xG0", &mut buffer).is_err());
        decode_tsv_field_into(br"good\x20again", &mut buffer).unwrap();
        assert_eq!(buffer, b"good again");
        assert_eq!(buffer.as_ptr(), original);
    }

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
    fn copies_long_plain_spans_around_sparse_escapes() {
        let input = [
            vec![b'a'; 16 * 1024],
            br"\t".to_vec(),
            vec![b'b'; 16 * 1024],
        ]
        .concat();
        let decoded = decode_tsv_field(&input).unwrap();
        assert_eq!(decoded.len(), 32 * 1024 + 1);
        assert_eq!(decoded[16 * 1024], b'\t');
        assert!(decoded[..16 * 1024].iter().all(|&byte| byte == b'a'));
        assert!(decoded[16 * 1024 + 1..].iter().all(|&byte| byte == b'b'));
    }

    #[test]
    fn plain_tsv_fields_keep_binary_bytes() {
        let input = [b'a', b'\t', b'\0', 0xff];
        assert_eq!(decode_tsv_field(&input).unwrap(), input);
        assert_eq!(decode_tsv_field(b"").unwrap(), b"");
    }

    #[test]
    fn rejects_invalid_escapes() {
        for input in [br"\x0".as_slice(), br"\q".as_slice(), br"\".as_slice()] {
            assert!(decode_tsv_field(input).is_err());
        }
    }
    #[test]
    fn sparse_escaped_tsv_uses_plain_spans_without_losing_adjacent_escapes() {
        let input = [vec![b'z'; 65_536], br"\t\n".to_vec(), vec![b'y'; 65_536]].concat();
        let decoded = decode_tsv_field(&input).unwrap();
        assert_eq!(decoded.len(), 131_074);
        assert_eq!(&decoded[65_536..65_538], b"\t\n");
        assert_eq!(decoded[0], b'z');
        assert_eq!(*decoded.last().unwrap(), b'y');
    }
}

#[cfg(test)]
mod zero_allocation_validation_tests {
    use super::*;

    #[test]
    fn validates_exactly_the_same_escape_grammar_as_the_decoder() {
        for field in [
            br"simple".as_slice(), br"".as_slice(), br"\t\n\r\0".as_slice(),
            br"a\x00b\x7F\\c".as_slice(), br"\b\f\v".as_slice(),
        ] {
            validate_tsv_field(field).unwrap();
            decode_tsv_field(field).unwrap();
        }
        for field in [
            br"\q".as_slice(), br"ending\".as_slice(),
            br"\x0".as_slice(), br"\xGG".as_slice(), br"hi\x".as_slice(),
        ] {
            assert!(validate_tsv_field(field).is_err());
            assert!(decode_tsv_field(field).is_err());
        }
        let mut long = vec![b'z'; 128 * 1024];
        long.extend_from_slice(br"\t");
        long.extend_from_slice(&vec![b'y'; 128 * 1024]);
        validate_tsv_field(&long).unwrap();
    }
}
