//! Exact global UUID MIN/MAX over independently validated peer aggregates.
//! UUID has a fixed 128-bit ordering; comparing formatted strings would also
//! work for canonical UUIDs, but accepting mixed-case hex would be incorrect.

use super::{AggregateFunction, MergeError};

fn uuid_value(raw: &[u8]) -> Option<u128> {
    if raw.len() != 36 {
        return None;
    }
    let mut number = 0u128;
    for (index, &byte) in raw.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if byte != b'-' {
                return None;
            }
            continue;
        }
        let hex = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        number = (number << 4) | u128::from(hex);
    }
    Some(number)
}

fn format_uuid(value: u128) -> String {
    let hex = format!("{value:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

pub(super) fn merge_uuid_extrema(
    shards: &[(usize, Vec<u8>)],
    function: AggregateFunction,
) -> Result<Vec<u8>, MergeError> {
    if !matches!(function, AggregateFunction::Min | AggregateFunction::Max) || shards.is_empty() {
        return Err(MergeError::InvalidExtremum);
    }
    let mut header = None;
    let mut best = None::<u128>;
    for (_, body) in shards {
        if !body.ends_with(b"\n") {
            return Err(MergeError::InvalidExtremum);
        }
        let split = body
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or(MergeError::InvalidExtremum)?;
        let next_header = &body[..=split];
        if next_header.len() <= 1 {
            return Err(MergeError::InvalidExtremum);
        }
        if let Some(expected) = header {
            if expected != next_header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            header = Some(next_header);
        }
        let raw = &body[split + 1..body.len() - 1];
        if raw == b"NULL" {
            continue;
        }
        let value = uuid_value(raw).ok_or(MergeError::InvalidExtremum)?;
        best = Some(best.map_or(value, |previous| match function {
            AggregateFunction::Min => previous.min(value),
            AggregateFunction::Max => previous.max(value),
            _ => unreachable!("validated aggregate"),
        }));
    }
    let mut result = header.ok_or(MergeError::InvalidExtremum)?.to_vec();
    if let Some(best) = best {
        result.extend_from_slice(format_uuid(best).as_bytes());
    } else {
        result.extend_from_slice(b"NULL");
    }
    result.push(b'\n');
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_uuid_extrema_by_binary_value_across_shards() {
        let shards = [
            (10, b"min(id)\n550E8400-E29B-41D4-A716-446655440000\n".to_vec()),
            (10, b"min(id)\n00000000-0000-0000-0000-000000000001\n".to_vec()),
            (10, b"min(id)\nNULL\n".to_vec()),
        ];
        assert_eq!(
            merge_uuid_extrema(&shards, AggregateFunction::Min).unwrap(),
            b"min(id)\n00000000-0000-0000-0000-000000000001\n"
        );
        let max = [
            (10, b"max(id)\nffffffff-ffff-ffff-ffff-ffffffffffff\n".to_vec()),
            (10, b"max(id)\n00000000-0000-0000-0000-000000000001\n".to_vec()),
        ];
        assert_eq!(
            merge_uuid_extrema(&max, AggregateFunction::Max).unwrap(),
            b"max(id)\nffffffff-ffff-ffff-ffff-ffffffffffff\n"
        );
    }

    #[test]
    fn all_null_shards_keep_the_single_empty_aggregate_result() {
        let shards = [(10, b"min(id)\nNULL\n".to_vec()), (10, b"min(id)\nNULL\n".to_vec())];
        assert_eq!(
            merge_uuid_extrema(&shards, AggregateFunction::Min).unwrap(),
            b"min(id)\nNULL\n"
        );
    }

    #[test]
    fn malformed_uuid_and_multiple_rows_fail_instead_of_partial_result() {
        for body in [
            b"min(id)\n550e8400e29b41d4a716446655440000\n".as_slice(),
            b"min(id)\n550e8400-e29b-41d4-a716-44665544000Z\n",
            b"min(id)\n00000000-0000-0000-0000-000000000000\nNULL\n",
            b"min(id)\n",
        ] {
            assert_eq!(
                merge_uuid_extrema(&[(10, body.to_vec())], AggregateFunction::Min),
                Err(MergeError::InvalidExtremum),
            );
        }
    }
}
