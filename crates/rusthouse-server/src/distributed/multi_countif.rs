//! Global grouped COUNTIF across multi-dimensional integer columns.
//! Counts are unsigned regardless of the source predicate value type.

use super::{MAX_RESPONSE_BYTES, MergeError, NormalizedGroupKey, normalized_group_key};
use std::collections::HashMap;

pub(super) fn merge_multi_key_countif(
    shards: &[(usize, Vec<u8>)],
    columns: &[String],
    value_column: &str,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_KEYS_BYTES: usize = 64 * 1024 * 1024;
    if !(2..=8).contains(&columns.len()) {
        return Err(MergeError::InvalidGroupedSum);
    }
    let header = format!("{}\tcountIf({value_column})\n", columns.join("\t"));
    let mut positions = HashMap::<Vec<NormalizedGroupKey<'_>>, usize>::new();
    let mut groups = Vec::<(&[u8], u128)>::new();
    let mut key_bytes = 0usize;
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
            let value = std::str::from_utf8(fields[columns.len()])
                .map_err(|_| MergeError::InvalidGroupedSum)?;
            let partial = value.parse::<u128>().map_err(|_| MergeError::InvalidGroupedSum)?;
            if partial.to_string() != value {
                return Err(MergeError::InvalidGroupedSum);
            }
            let normalized = fields[..columns.len()]
                .iter()
                .map(|field| normalized_group_key(field)
                    .map_err(|_| MergeError::InvalidGroupedSum))
                .collect::<Result<Vec<_>, _>>()?;
            let tuple_len = row.len()
                .checked_sub(value.len() + 1)
                .ok_or(MergeError::InvalidGroupedSum)?;
            if let Some(&id) = positions.get(&normalized) {
                groups[id].1 = groups[id].1
                    .checked_add(partial)
                    .ok_or(MergeError::CountOverflow)?;
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                key_bytes = key_bytes
                    .checked_add(tuple_len)
                    .filter(|&length| length <= MAX_KEYS_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let id = groups.len();
                positions.insert(normalized, id);
                groups.push((&row[..tuple_len], partial));
            }
        }
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = header.into_bytes();
    for (tuple, count) in groups {
        let digits = count.to_string();
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
    fn combines_signed_numeric_predicate_counts_on_shared_tuples() {
        let shards = [
            (10, b"region\tcategory\tcountIf(amount)\nwest\ta\t2\nwest\tb\t0\n".to_vec()),
            (10, b"region\tcategory\tcountIf(amount)\nwest\ta\t3\neast\tb\t1\n".to_vec()),
        ];
        assert_eq!(
            merge_multi_key_countif(&shards, &dimensions(), "amount", 10).unwrap(),
            b"region\tcategory\tcountIf(amount)\nwest\ta\t5\nwest\tb\t0\neast\tb\t1\n"
        );
    }

    #[test]
    fn uses_exact_integer_counts_without_losing_large_partials() {
        let prefix = "region\tcategory\tcountIf(amount)\n";
        let shards = [
            (10, format!("{prefix}west\ta\t{}\n", u128::MAX - 1).into_bytes()),
            (10, format!("{prefix}west\ta\t1\n").into_bytes()),
        ];
        assert_eq!(
            merge_multi_key_countif(&shards, &dimensions(), "amount", 10).unwrap(),
            format!("{prefix}west\ta\t{}\n", u128::MAX).into_bytes()
        );
        let overflow = [
            (10, format!("{prefix}west\ta\t{}\n", u128::MAX).into_bytes()),
            (10, format!("{prefix}west\ta\t1\n").into_bytes()),
        ];
        assert_eq!(
            merge_multi_key_countif(&overflow, &dimensions(), "amount", 10),
            Err(MergeError::CountOverflow)
        );
    }

    #[test]
    fn rejects_malformed_counts_and_peer_row_caps() {
        let prefix = "region\tcategory\tcountIf(amount)\n";
        for number in ["-1", "NULL", "01", "not-count"] {
            assert_eq!(
                merge_multi_key_countif(
                    &[(10, format!("{prefix}west\ta\t{number}\n").into_bytes())],
                    &dimensions(), "amount", 10
                ),
                Err(MergeError::InvalidGroupedSum),
            );
        }
        assert_eq!(
            merge_multi_key_countif(
                &[(1, format!("{prefix}west\ta\t1\n").into_bytes())],
                &dimensions(), "amount", 10
            ),
            Err(MergeError::ShardRowLimit)
        );
    }
}
