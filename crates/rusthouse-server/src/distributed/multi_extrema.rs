//! Exact multi-key integer MIN/MAX across disjoint shards.
//! Range-check every partial against its physical declared SQL type.

use super::{AggregateFunction, MAX_RESPONSE_BYTES, MergeError, NormalizedGroupKey,
    extrema_fits_declared_type, normalized_group_key};
use std::collections::HashMap;

pub(super) fn merge_multi_key_extrema(
    shards: &[(usize, Vec<u8>)],
    columns: &[String],
    value_column: &str,
    kind: &str,
    function: AggregateFunction,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_GROUP_KEY_BYTES: usize = 64 * 1024 * 1024;
    if !(2..=8).contains(&columns.len())
        || !matches!(function, AggregateFunction::Min | AggregateFunction::Max)
    {
        return Err(MergeError::InvalidGroupedSum);
    }
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let header = format!("{}\t{}({value_column})\n", columns.join("\t"), function.name());
    let mut positions = HashMap::<Vec<NormalizedGroupKey<'_>>, usize>::new();
    let mut groups = Vec::<(&[u8], Option<i128>, Option<u128>)>::new();
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
            let normalized = fields[..columns.len()]
                .iter()
                .map(|field| normalized_group_key(field)
                    .map_err(|_| MergeError::InvalidGroupedSum))
                .collect::<Result<Vec<_>, _>>()?;
            let wire = fields[columns.len()];
            let key_len = row.len()
                .checked_sub(wire.len() + 1)
                .ok_or(MergeError::InvalidGroupedSum)?;
            let index = if let Some(&id) = positions.get(&normalized) {
                id
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                key_bytes = key_bytes.checked_add(key_len)
                    .filter(|&total| total <= MAX_GROUP_KEY_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let id = groups.len();
                positions.insert(normalized, id);
                groups.push((&row[..key_len], None, None));
                id
            };
            if wire == b"NULL" {
                continue;
            }
            let text = std::str::from_utf8(wire).map_err(|_| MergeError::InvalidGroupedSum)?;
            if !extrema_fits_declared_type(kind, text) {
                return Err(MergeError::InvalidGroupedSum);
            }
            if unsigned {
                let value = text.parse::<u128>().map_err(|_| MergeError::InvalidGroupedSum)?;
                let prior = &mut groups[index].2;
                *prior = Some(prior.map_or(value, |old| match function {
                    AggregateFunction::Min => old.min(value),
                    AggregateFunction::Max => old.max(value),
                    _ => unreachable!("validated extrema"),
                }));
            } else {
                let value = text.parse::<i128>().map_err(|_| MergeError::InvalidGroupedSum)?;
                let prior = &mut groups[index].1;
                *prior = Some(prior.map_or(value, |old| match function {
                    AggregateFunction::Min => old.min(value),
                    AggregateFunction::Max => old.max(value),
                    _ => unreachable!("validated extrema"),
                }));
            }
        }
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = header.into_bytes();
    for (key, signed, unsigned_value) in groups {
        let rendered = if unsigned {
            unsigned_value.map(|value| value.to_string())
        } else {
            signed.map(|value| value.to_string())
        }
        .unwrap_or_else(|| "NULL".to_owned());
        if output
            .len()
            .checked_add(key.len() + rendered.len() + 2)
            .is_none_or(|bytes| bytes > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(key);
        output.push(b'\t');
        output.extend_from_slice(rendered.as_bytes());
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
    fn combines_signed_extrema_and_nullable_groups_across_peers() {
        let shards = [
            (10, b"region\tcategory\tmin(amount)\nwest\ta\t-4\neast\tb\tNULL\n".to_vec()),
            (10, b"region\tcategory\tmin(amount)\nwest\ta\t-9\neast\tb\t5\n".to_vec()),
        ];
        assert_eq!(
            merge_multi_key_extrema(
                &shards, &dimensions(), "amount", "Nullable(Int64)", AggregateFunction::Min, 10
            ).unwrap(),
            b"region\tcategory\tmin(amount)\nwest\ta\t-9\neast\tb\t5\n"
        );
        let max = [
            (10, b"region\tcategory\tmax(amount)\nwest\ta\t1\n".to_vec()),
            (10, b"region\tcategory\tmax(amount)\nwest\ta\t9\n".to_vec()),
        ];
        assert_eq!(
            merge_multi_key_extrema(
                &max, &dimensions(), "amount", "UInt8", AggregateFunction::Max, 10
            ).unwrap(),
            b"region\tcategory\tmax(amount)\nwest\ta\t9\n"
        );
    }

    #[test]
    fn rejects_values_outside_physical_type_and_malformed_tuples() {
        let prefix = "region\tcategory\tmax(amount)\n";
        assert_eq!(
            merge_multi_key_extrema(
                &[(10, format!("{prefix}west\ta\t256\n").into_bytes())],
                &dimensions(), "amount", "UInt8", AggregateFunction::Max, 10
            ),
            Err(MergeError::InvalidGroupedSum)
        );
        assert_eq!(
            merge_multi_key_extrema(
                &[(10, format!("{prefix}west\t1\n").into_bytes())],
                &dimensions(), "amount", "Int64", AggregateFunction::Max, 10
            ),
            Err(MergeError::InvalidGroupedSum)
        );
        assert_eq!(
            merge_multi_key_extrema(
                &[(1, format!("{prefix}west\ta\t1\n").into_bytes())],
                &dimensions(), "amount", "Int64", AggregateFunction::Max, 10
            ),
            Err(MergeError::ShardRowLimit)
        );
    }
}
