//! Exact multi-dimensional GROUP BY PRODUCT over integer partials.
//! Unlike SUM, PRODUCT has a multiplicative identity and signed overflow.

use super::{MAX_RESPONSE_BYTES, MergeError, NormalizedGroupKey, normalized_group_key};
use std::collections::HashMap;

pub(super) fn merge_multi_key_product(
    shards: &[(usize, Vec<u8>)],
    columns: &[String],
    value_column: &str,
    kind: &str,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_KEYS_BYTES: usize = 64 * 1024 * 1024;
    if !(2..=8).contains(&columns.len()) {
        return Err(MergeError::InvalidGroupedProduct);
    }
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let header = format!("{}\tproduct({value_column})\n", columns.join("\t"));
    let mut lookup = HashMap::<Vec<NormalizedGroupKey<'_>>, usize>::new();
    let mut groups = Vec::<(&[u8], Option<i128>, Option<u128>)>::new();
    let mut retained_bytes = 0usize;
    for (cap, body) in shards {
        if *cap == 0 || !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidGroupedProduct);
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
                return Err(MergeError::InvalidGroupedProduct);
            }
            let normalized = fields[..columns.len()]
                .iter()
                .map(|field| normalized_group_key(field)
                    .map_err(|_| MergeError::InvalidGroupedProduct))
                .collect::<Result<Vec<_>, _>>()?;
            let raw = fields[columns.len()];
            let key_len = row.len()
                .checked_sub(raw.len() + 1)
                .ok_or(MergeError::InvalidGroupedProduct)?;
            let index = if let Some(&id) = lookup.get(&normalized) {
                id
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                retained_bytes = retained_bytes
                    .checked_add(key_len)
                    .filter(|&length| length <= MAX_KEYS_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let id = groups.len();
                lookup.insert(normalized, id);
                groups.push((&row[..key_len], None, None));
                id
            };
            if raw == b"NULL" {
                continue;
            }
            let text = std::str::from_utf8(raw)
                .map_err(|_| MergeError::InvalidGroupedProduct)?;
            if unsigned {
                let value = text.parse::<u128>()
                    .map_err(|_| MergeError::InvalidGroupedProduct)?;
                groups[index].2 = Some(match groups[index].2 {
                    None => value,
                    Some(previous) => previous.checked_mul(value)
                        .ok_or(MergeError::ProductOverflow)?,
                });
            } else {
                let value = text.parse::<i128>()
                    .map_err(|_| MergeError::InvalidGroupedProduct)?;
                groups[index].1 = Some(match groups[index].1 {
                    None => value,
                    Some(previous) => previous.checked_mul(value)
                        .ok_or(MergeError::ProductOverflow)?,
                });
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
            .is_none_or(|length| length > MAX_RESPONSE_BYTES)
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
    fn multiplies_shared_signed_groups_and_preserves_null_groups() {
        let shards = [
            (10, b"region\tcategory\tproduct(amount)\nwest\ta\t-3\neast\tb\tNULL\n".to_vec()),
            (10, b"region\tcategory\tproduct(amount)\nwest\ta\t4\neast\tb\t5\n".to_vec()),
        ];
        assert_eq!(
            merge_multi_key_product(&shards, &dimensions(), "amount", "Int64", 10).unwrap(),
            b"region\tcategory\tproduct(amount)\nwest\ta\t-12\neast\tb\t5\n"
        );
    }

    #[test]
    fn zero_factor_wins_and_unsigned_overflow_is_rejected() {
        let prefix = "region\tcategory\tproduct(amount)\n";
        let with_zero = [
            (10, format!("{prefix}west\ta\t0\n").into_bytes()),
            (10, format!("{prefix}west\ta\t{}\n", u128::MAX).into_bytes()),
        ];
        assert_eq!(
            merge_multi_key_product(&with_zero, &dimensions(), "amount", "UInt128", 10).unwrap(),
            format!("{prefix}west\ta\t0\n").into_bytes()
        );
        let overflow = [
            (10, format!("{prefix}west\ta\t{}\n", u128::MAX).into_bytes()),
            (10, format!("{prefix}west\ta\t2\n").into_bytes()),
        ];
        assert_eq!(
            merge_multi_key_product(&overflow, &dimensions(), "amount", "UInt128", 10),
            Err(MergeError::ProductOverflow)
        );
    }

    #[test]
    fn malformed_product_and_peer_caps_are_fail_closed() {
        let prefix = "region\tcategory\tproduct(amount)\n";
        assert_eq!(
            merge_multi_key_product(
                &[(10, format!("{prefix}west\ta\tbad\n").into_bytes())],
                &dimensions(), "amount", "Int64", 10
            ),
            Err(MergeError::InvalidGroupedProduct)
        );
        assert_eq!(
            merge_multi_key_product(
                &[(1, format!("{prefix}west\ta\t4\n").into_bytes())],
                &dimensions(), "amount", "Int64", 10
            ),
            Err(MergeError::ShardRowLimit)
        );
    }
}
