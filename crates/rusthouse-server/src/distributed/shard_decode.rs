//! Validate single-row scalar TSV shard results before aggregate reduction.
use super::MergeError;

/// Borrowed scalar response. The header includes its terminating newline.
pub(super) struct ShardScalarRow<'a> {
    pub header: &'a [u8],
    pub value: &'a [u8],
}

/// Parse the common scalar response envelope without allocating.
/// Each reducer retains its own numeric-domain and NULL validation.
pub(super) fn decode_scalar_row(
    body: &[u8],
    invalid: MergeError,
) -> Result<ShardScalarRow<'_>, MergeError> {
    if !body.ends_with(b"\n") {
        return Err(invalid);
    }
    let Some(end) = body.iter().position(|&byte| byte == b'\n') else {
        return Err(invalid);
    };
    let header = &body[..=end];
    if header.len() <= 1 || body.len() <= end + 1 {
        return Err(invalid);
    }
    Ok(ShardScalarRow {
        header,
        value: &body[end + 1..body.len() - 1],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_valid_scalar_envelope() {
        let row = decode_scalar_row(b"sum(x)\n123\n", MergeError::InvalidSum).unwrap();
        assert_eq!(row.header, b"sum(x)\n");
        assert_eq!(row.value, b"123");
    }

    #[test]
    fn refuses_missing_or_truncated_scalar_rows() {
        for response in [b"sum(x)\n".as_slice(), b"sum(x)\n123", b"\n123\n"] {
            assert!(matches!(
                decode_scalar_row(response, MergeError::InvalidCount),
                Err(MergeError::InvalidCount)
            ));
        }
    }

    #[test]
    fn preserves_empty_value_for_reducer_validation() {
        let row = decode_scalar_row(b"sum(x)\n\n", MergeError::InvalidSum).unwrap();
        assert!(row.value.is_empty());
    }
}
