//! Exact global multi-dimensional integer SUM across shard-local groups.
//! NULL sums do not erase non-NULL contributions from other shards.

use super::{
    MAX_RESPONSE_BYTES, MergeError, NormalizedGroupKey, WideSignedSum, normalized_group_key,
};
use std::collections::HashMap;

pub(super) fn merge_multi_key_sums(
    shards: &[(usize, Vec<u8>)],
    columns: &[String],
    value_column: &str,
    kind: &str,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_KEYS_BYTES: usize = 64 * 1024 * 1024;
    if !(2..=8).contains(&columns.len()) {
        return Err(MergeError::InvalidGroupedSum);
    }
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let header = format!("{}\tsum({value_column})\n", columns.join("\t"));
    let mut lookup = HashMap::<Vec<NormalizedGroupKey<'_>>, usize>::new();
    let mut groups = Vec::<(&[u8], Option<WideSignedSum>, Option<u128>)>::new();
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
            count = count
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if count >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            let fields = row.split(|&byte| byte == b'\t').collect::<Vec<_>>();
            if fields.len() != columns.len() + 1 {
                return Err(MergeError::InvalidGroupedSum);
            }
            let normalized = fields[..columns.len()]
                .iter()
                .map(|field| {
                    normalized_group_key(field).map_err(|_| MergeError::InvalidGroupedSum)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let wire = fields[columns.len()];
            let key_len = row
                .len()
                .checked_sub(wire.len() + 1)
                .ok_or(MergeError::InvalidGroupedSum)?;
            let id = if let Some(&id) = lookup.get(&normalized) {
                id
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                retained_bytes = retained_bytes
                    .checked_add(key_len)
                    .filter(|&bytes| bytes <= MAX_KEYS_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let id = groups.len();
                lookup.insert(normalized, id);
                groups.push((&row[..key_len], None, None));
                id
            };
            if wire == b"NULL" {
                continue;
            }
            let digits = std::str::from_utf8(wire).map_err(|_| MergeError::InvalidGroupedSum)?;
            if unsigned {
                let number = digits
                    .parse::<u128>()
                    .map_err(|_| MergeError::InvalidGroupedSum)?;
                groups[id].2 = Some(groups[id].2
                    .unwrap_or(0)
                    .checked_add(number)
                    .ok_or(MergeError::SumOverflow)?);
            } else {
                let number = digits
                    .parse::<i128>()
                    .map_err(|_| MergeError::InvalidGroupedSum)?;
                if let Some(sum) = &mut groups[id].1 {
                    sum.add(number)?;
                } else {
                    groups[id].1 = Some(WideSignedSum::new(number));
                }
            }
        }
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = header.into_bytes();
    for (key, signed, unsigned_sum) in groups {
        let value = if unsigned {
            unsigned_sum.map(|value| value.to_string())
        } else {
            signed.map(WideSignedSum::to_i128)
                .transpose()?
                .map(|value| value.to_string())
        }
        .unwrap_or_else(|| "NULL".to_owned());
        if output
            .len()
            .checked_add(key.len() + value.len() + 2)
            .is_none_or(|bytes| bytes > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(key);
        output.push(b'\t');
        output.extend_from_slice(value.as_bytes());
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
    fn sums_shared_composite_keys_and_retains_null_only_groups() {
        let shards = [
            (10, b"region\tcategory\tsum(amount)\nwest\ta\t4\neast\tb\tNULL\n".to_vec()),
            (10, b"region\tcategory\tsum(amount)\nwest\ta\t-2\neast\tb\t7\n".to_vec()),
        ];
        assert_eq!(
            merge_multi_key_sums(&shards, &dimensions(), "amount", "Int64", 10).unwrap(),
            b"region\tcategory\tsum(amount)\nwest\ta\t2\neast\tb\t7\n"
        );
    }

    #[test]
    fn signed_intermediate_overflow_can_cancel_across_shards() {
        let prefix = "region\tcategory\tsum(amount)\n";
        let shards = [
            (10, format!("{prefix}west\ta\t{}\n", i128::MAX).into_bytes()),
            (10, format!("{prefix}west\ta\t1\n").into_bytes()),
            (10, format!("{prefix}west\ta\t-1\n").into_bytes()),
        ];
        assert_eq!(
            merge_multi_key_sums(&shards, &dimensions(), "amount", "Int128", 10).unwrap(),
            format!("{prefix}west\ta\t{}\n", i128::MAX).into_bytes()
        );
    }

    #[test]
    fn detects_unsigned_overflow_wrong_width_and_capped_shards() {
        let prefix = "region\tcategory\tsum(amount)\n";
        let overflow = [
            (10, format!("{prefix}west\ta\t{}\n", u128::MAX).into_bytes()),
            (10, format!("{prefix}west\ta\t1\n").into_bytes()),
        ];
        assert_eq!(
            merge_multi_key_sums(&overflow, &dimensions(), "amount", "UInt128", 10),
            Err(MergeError::SumOverflow),
        );
        assert_eq!(
            merge_multi_key_sums(
                &[(10, format!("{prefix}west\t1\n").into_bytes())],
                &dimensions(), "amount", "UInt64", 10,
            ),
            Err(MergeError::InvalidGroupedSum)
        );
        assert_eq!(
            merge_multi_key_sums(
                &[(1, format!("{prefix}west\ta\t1\n").into_bytes())],
                &dimensions(), "amount", "Int64", 10,
            ),
            Err(MergeError::ShardRowLimit)
        );
    }
}
