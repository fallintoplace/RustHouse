//! Exact multi-key GROUP BY bitwise aggregates over partial integer states.
//! Preserve NULL if every peer has only NULL values for a group.

use super::{AggregateFunction, MAX_RESPONSE_BYTES, MergeError,
    NormalizedGroupKey, normalized_group_key};
use std::collections::HashMap;

pub(super) fn merge_multi_key_bitwise(
    shards: &[(usize, Vec<u8>)],
    columns: &[String],
    value_column: &str,
    kind: &str,
    function: AggregateFunction,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_KEY_BYTES: usize = 64 * 1024 * 1024;
    if !(2..=8).contains(&columns.len())
        || !matches!(function,
            AggregateFunction::GroupBitAnd
            | AggregateFunction::GroupBitOr
            | AggregateFunction::GroupBitXor)
    {
        return Err(MergeError::InvalidGroupedBitwise);
    }
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let header = format!("{}\t{}({value_column})\n", columns.join("\t"), function.name());
    let mut lookup = HashMap::<Vec<NormalizedGroupKey<'_>>, usize>::new();
    let mut groups = Vec::<(&[u8], Option<i128>, Option<u128>)>::new();
    let mut retained_bytes = 0usize;
    for (cap, body) in shards {
        if *cap == 0 || !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidGroupedBitwise);
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
                return Err(MergeError::InvalidGroupedBitwise);
            }
            let tuple = fields[..columns.len()]
                .iter()
                .map(|field| normalized_group_key(field)
                    .map_err(|_| MergeError::InvalidGroupedBitwise))
                .collect::<Result<Vec<_>, _>>()?;
            let raw = fields[columns.len()];
            let tuple_len = row.len()
                .checked_sub(raw.len() + 1)
                .ok_or(MergeError::InvalidGroupedBitwise)?;
            let index = if let Some(&id) = lookup.get(&tuple) {
                id
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                retained_bytes = retained_bytes
                    .checked_add(tuple_len)
                    .filter(|&bytes| bytes <= MAX_KEY_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let id = groups.len();
                lookup.insert(tuple, id);
                groups.push((&row[..tuple_len], None, None));
                id
            };
            if raw == b"NULL" {
                continue;
            }
            let digits = std::str::from_utf8(raw)
                .map_err(|_| MergeError::InvalidGroupedBitwise)?;
            if unsigned {
                let value = digits.parse::<u128>()
                    .map_err(|_| MergeError::InvalidGroupedBitwise)?;
                groups[index].2 = Some(match groups[index].2 {
                    None => value,
                    Some(prior) => match function {
                        AggregateFunction::GroupBitAnd => prior & value,
                        AggregateFunction::GroupBitOr => prior | value,
                        AggregateFunction::GroupBitXor => prior ^ value,
                        _ => unreachable!("validated bitwise aggregate"),
                    },
                });
            } else {
                let value = digits.parse::<i128>()
                    .map_err(|_| MergeError::InvalidGroupedBitwise)?;
                groups[index].1 = Some(match groups[index].1 {
                    None => value,
                    Some(prior) => match function {
                        AggregateFunction::GroupBitAnd => prior & value,
                        AggregateFunction::GroupBitOr => prior | value,
                        AggregateFunction::GroupBitXor => prior ^ value,
                        _ => unreachable!("validated bitwise aggregate"),
                    },
                });
            }
        }
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = header.into_bytes();
    for (tuple, signed, unsigned_value) in groups {
        let value = if unsigned {
            unsigned_value.map(|value| value.to_string())
        } else {
            signed.map(|value| value.to_string())
        }
        .unwrap_or_else(|| "NULL".to_owned());
        if output
            .len()
            .checked_add(tuple.len() + value.len() + 2)
            .is_none_or(|len| len > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(tuple);
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
    fn combines_xor_or_and_and_with_shared_tuples() {
        let xor = [
            (10, b"region\tcategory\tgroupBitXor(flags)\nwest\ta\t7\neast\tb\tNULL\n".to_vec()),
            (10, b"region\tcategory\tgroupBitXor(flags)\nwest\ta\t3\neast\tb\t5\n".to_vec()),
        ];
        assert_eq!(
            merge_multi_key_bitwise(
                &xor, &dimensions(), "flags", "UInt64", AggregateFunction::GroupBitXor, 10
            ).unwrap(),
            b"region\tcategory\tgroupBitXor(flags)\nwest\ta\t4\neast\tb\t5\n"
        );
        for (function, label, result) in [
            (AggregateFunction::GroupBitAnd, "groupBitAnd", 2),
            (AggregateFunction::GroupBitOr, "groupBitOr", 7),
        ] {
            let shards = [
                (10, format!("region\tcategory\t{label}(flags)\nwest\ta\t3\n").into_bytes()),
                (10, format!("region\tcategory\t{label}(flags)\nwest\ta\t6\n").into_bytes()),
            ];
            assert_eq!(
                merge_multi_key_bitwise(
                    &shards, &dimensions(), "flags", "UInt32", function, 10
                ).unwrap(),
                format!("region\tcategory\t{label}(flags)\nwest\ta\t{result}\n").into_bytes()
            );
        }
    }

    #[test]
    fn rejects_corrupt_partial_values_and_saturated_shards() {
        let prefix = "region\tcategory\tgroupBitXor(flags)\n";
        assert_eq!(
            merge_multi_key_bitwise(
                &[(10, format!("{prefix}west\ta\t-1\n").into_bytes())],
                &dimensions(), "flags", "UInt64", AggregateFunction::GroupBitXor, 10
            ),
            Err(MergeError::InvalidGroupedBitwise)
        );
        assert_eq!(
            merge_multi_key_bitwise(
                &[(1, format!("{prefix}west\ta\t7\n").into_bytes())],
                &dimensions(), "flags", "UInt64", AggregateFunction::GroupBitXor, 10
            ),
            Err(MergeError::ShardRowLimit)
        );
    }
}
