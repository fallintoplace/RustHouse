//! Exact multi-key COUNT(column) across all local data types.
//! Local peers already exclude NULL input values, so partials are unsigned.

use super::{MAX_RESPONSE_BYTES, MergeError, NormalizedGroupKey, normalized_group_key};
use std::collections::HashMap;

pub(super) fn merge_multi_key_count_column(
    shards: &[(usize, Vec<u8>)],
    columns: &[String],
    value_column: &str,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_KEY_BYTES: usize = 64 * 1024 * 1024;
    if !(2..=8).contains(&columns.len()) {
        return Err(MergeError::InvalidGroupedCount);
    }
    let header = format!("{}\tcount({value_column})\n", columns.join("\t"));
    let mut lookup = HashMap::<Vec<NormalizedGroupKey<'_>>, usize>::new();
    let mut groups = Vec::<(&[u8], u128)>::new();
    let mut retained_bytes = 0usize;
    for (cap, body) in shards {
        if *cap == 0 || !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidGroupedCount);
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
                return Err(MergeError::InvalidGroupedCount);
            }
            let digits = std::str::from_utf8(fields[columns.len()])
                .map_err(|_| MergeError::InvalidGroupedCount)?;
            let partial = digits.parse::<u128>()
                .map_err(|_| MergeError::InvalidGroupedCount)?;
            if partial.to_string() != digits {
                return Err(MergeError::InvalidGroupedCount);
            }
            let tuple = fields[..columns.len()]
                .iter()
                .map(|field| normalized_group_key(field)
                    .map_err(|_| MergeError::InvalidGroupedCount))
                .collect::<Result<Vec<_>, _>>()?;
            let tuple_len = row.len()
                .checked_sub(digits.len() + 1)
                .ok_or(MergeError::InvalidGroupedCount)?;
            if let Some(&id) = lookup.get(&tuple) {
                groups[id].1 = groups[id].1
                    .checked_add(partial)
                    .ok_or(MergeError::CountOverflow)?;
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                retained_bytes = retained_bytes
                    .checked_add(tuple_len)
                    .filter(|&len| len <= MAX_KEY_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let id = groups.len();
                lookup.insert(tuple, id);
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
            .is_none_or(|len| len > MAX_RESPONSE_BYTES)
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
    fn merges_non_null_string_counts_across_composite_keys() {
        let shards = [
            (10, b"region\tcategory\tcount(label)\nwest\ta\t2\neast\tb\t0\n".to_vec()),
            (10, b"region\tcategory\tcount(label)\nwest\ta\t3\nwest\tb\t1\n".to_vec()),
        ];
        assert_eq!(
            merge_multi_key_count_column(&shards, &dimensions(), "label", 10).unwrap(),
            b"region\tcategory\tcount(label)\nwest\ta\t5\neast\tb\t0\nwest\tb\t1\n"
        );
    }

    #[test]
    fn rejects_negative_counts_corrupt_tuples_and_row_cap() {
        let prefix = "region\tcategory\tcount(label)\n";
        for suffix in ["west\ta\t-1\n", "west\ta\tNULL\n", "west\ta\t01\n", "west\t2\n"] {
            assert_eq!(
                merge_multi_key_count_column(
                    &[(10, format!("{prefix}{suffix}").into_bytes())],
                    &dimensions(), "label", 10
                ),
                Err(MergeError::InvalidGroupedCount)
            );
        }
        assert_eq!(
            merge_multi_key_count_column(
                &[(1, format!("{prefix}west\ta\t1\n").into_bytes())],
                &dimensions(), "label", 10
            ),
            Err(MergeError::ShardRowLimit)
        );
    }
}
