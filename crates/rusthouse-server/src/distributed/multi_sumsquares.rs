//! Exact global multi-dimensional sumSquares for integer values.
//! Every shard supplies a checked local partial; totals stay in the same
//! signed or unsigned 128-bit domain as local SQL.

use super::{MAX_RESPONSE_BYTES, MergeError, NormalizedGroupKey, normalized_group_key};
use std::collections::HashMap;

pub(super) fn merge_multi_key_sumsquares(
    shards: &[(usize, Vec<u8>)],
    columns: &[String],
    value_column: &str,
    kind: &str,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_GROUP_KEY_BYTES: usize = 64 * 1024 * 1024;
    if !(2..=8).contains(&columns.len()) {
        return Err(MergeError::InvalidGroupedSum);
    }
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let header = format!("{}\tsumSquares({value_column})\n", columns.join("\t"));
    let mut positions = HashMap::<Vec<NormalizedGroupKey<'_>>, usize>::new();
    let mut groups = Vec::<(&[u8], Option<u128>)>::new();
    let mut retained_bytes = 0usize;
    for (cap, body) in shards {
        if *cap == 0 || !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidGroupedSum);
        }
        let rows = &body[header.len()..];
        if rows.is_empty() {
            continue;
        }
        let mut count = 0usize;
        for row in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            count = count.checked_add(1).ok_or(MergeError::CoordinatorLimit)?;
            if count >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            let fields = row.split(|&byte| byte == b'\t').collect::<Vec<_>>();
            if fields.len() != columns.len() + 1 {
                return Err(MergeError::InvalidGroupedSum);
            }
            let normalized = fields[..columns.len()]
                .iter()
                .map(|field| normalized_group_key(field)
                    .map_err(|_| MergeError::InvalidGroupedSum))
                .collect::<Result<Vec<_>, _>>()?;
            let wire = fields[columns.len()];
            let key_len = row.len()
                .checked_sub(wire.len() + 1)
                .ok_or(MergeError::InvalidGroupedSum)?;
            let id = if let Some(&id) = positions.get(&normalized) {
                id
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                retained_bytes = retained_bytes
                    .checked_add(key_len)
                    .filter(|&length| length <= MAX_GROUP_KEY_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let id = groups.len();
                positions.insert(normalized, id);
                groups.push((&row[..key_len], None));
                id
            };
            if wire == b"NULL" {
                continue;
            }
            let value = std::str::from_utf8(wire)
                .map_err(|_| MergeError::InvalidGroupedSum)?
                .parse::<u128>()
                .map_err(|_| MergeError::InvalidGroupedSum)?;
            let next = groups[id].1
                .unwrap_or(0)
                .checked_add(value)
                .ok_or(MergeError::SumOverflow)?;
            if !unsigned && next > i128::MAX as u128 {
                return Err(MergeError::SumOverflow);
            }
            groups[id].1 = Some(next);
        }
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = header.into_bytes();
    for (tuple, value) in groups {
        let digits = value.map_or_else(|| "NULL".to_owned(), |value| value.to_string());
        if output
            .len()
            .checked_add(tuple.len() + digits.len() + 2)
            .is_none_or(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(tuple);
        output.push(b'\t');
        output.extend_from_slice(digits.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dimensions() -> Vec<String> {
        vec!["region".into(), "category".into()]
    }

    #[test]
    fn combines_nonnegative_squares_for_signed_inputs() {
        let shards = [
            (10, b"region\tcategory\tsumSquares(amount)\nwest\ta\t25\neast\tb\tNULL\n".to_vec()),
            (10, b"region\tcategory\tsumSquares(amount)\nwest\ta\t9\neast\tb\t16\n".to_vec()),
        ];
        assert_eq!(
            merge_multi_key_sumsquares(&shards, &dimensions(), "amount", "Int64", 10).unwrap(),
            b"region\tcategory\tsumSquares(amount)\nwest\ta\t34\neast\tb\t16\n"
        );
    }

    #[test]
    fn rejects_signed_128_bit_overflow_after_cross_shard_reduction() {
        let prefix = "region\tcategory\tsumSquares(amount)\n";
        let shards = [
            (10, format!("{prefix}west\ta\t{}\n", i128::MAX).into_bytes()),
            (10, format!("{prefix}west\ta\t1\n").into_bytes()),
        ];
        assert_eq!(
            merge_multi_key_sumsquares(&shards, &dimensions(), "amount", "Int128", 10),
            Err(MergeError::SumOverflow)
        );
    }

    #[test]
    fn rejects_negative_squares_malformed_tuples_and_peer_caps() {
        let prefix = "region\tcategory\tsumSquares(amount)\n";
        assert_eq!(
            merge_multi_key_sumsquares(
                &[(10, format!("{prefix}west\ta\t-1\n").into_bytes())],
                &dimensions(), "amount", "Int64", 10
            ),
            Err(MergeError::InvalidGroupedSum)
        );
        assert_eq!(
            merge_multi_key_sumsquares(
                &[(10, format!("{prefix}west\t9\n").into_bytes())],
                &dimensions(), "amount", "UInt64", 10
            ),
            Err(MergeError::InvalidGroupedSum)
        );
        assert_eq!(
            merge_multi_key_sumsquares(
                &[(1, format!("{prefix}west\ta\t9\n").into_bytes())],
                &dimensions(), "amount", "UInt64", 10
            ),
            Err(MergeError::ShardRowLimit)
        );
    }
}
