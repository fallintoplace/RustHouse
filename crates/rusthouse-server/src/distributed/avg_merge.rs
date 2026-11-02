//! Global weighted average from exact integer sums and non-NULL counts.
//! Averaging local averages would overweight small shards.

use super::{MergeError, WideSignedSum};

pub(super) fn merge_global_avg(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    kind: &str,
) -> Result<Vec<u8>, MergeError> {
    // Keep a safe exact integer accumulation domain before final f64 rounding.
    let kind = kind.strip_prefix("Nullable(")
        .and_then(|name| name.strip_suffix(')'))
        .unwrap_or(kind);
    let unsigned = match kind {
        "UInt8" | "UInt16" | "UInt32" | "UInt64" => true,
        "Int8" | "Int16" | "Int32" | "Int64" => false,
        _ => return Err(MergeError::InvalidSum),
    };
    let header = format!("sum({column})\tcount({column})\n");
    let mut unsigned_sum = 0u128;
    let mut signed_sum = None::<WideSignedSum>;
    let mut total = 0u128;

    if shards.is_empty() {
        return Err(MergeError::InvalidSum);
    }
    for (cap, body) in shards {
        if *cap == 0 || !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidSum);
        }
        let row = &body[header.len()..body.len() - 1];
        let Some(tab) = row.iter().position(|&byte| byte == b'\t') else {
            return Err(MergeError::InvalidSum);
        };
        let raw_sum = &row[..tab];
        let raw_count = &row[tab + 1..];
        if raw_count.contains(&b'\t') {
            return Err(MergeError::InvalidSum);
        }
        let count_text = std::str::from_utf8(raw_count).map_err(|_| MergeError::InvalidSum)?;
        let count = count_text.parse::<u128>().map_err(|_| MergeError::InvalidSum)?;
        if count.to_string() != count_text || (count == 0) != (raw_sum == b"NULL") {
            return Err(MergeError::InvalidSum);
        }
        total = total.checked_add(count).ok_or(MergeError::CountOverflow)?;
        if count == 0 {
            continue;
        }
        let digits = std::str::from_utf8(raw_sum).map_err(|_| MergeError::InvalidSum)?;
        if unsigned {
            let value = digits.parse::<u128>().map_err(|_| MergeError::InvalidSum)?;
            unsigned_sum = unsigned_sum.checked_add(value).ok_or(MergeError::SumOverflow)?;
        } else {
            let value = digits.parse::<i128>().map_err(|_| MergeError::InvalidSum)?;
            if let Some(previous) = &mut signed_sum {
                previous.add(value)?;
            } else {
                signed_sum = Some(WideSignedSum::new(value));
            }
        }
    }
    let rendered = if total == 0 {
        "NULL".to_owned()
    } else {
        let numerator = if unsigned {
            unsigned_sum as f64
        } else {
            signed_sum.ok_or(MergeError::InvalidSum)?.to_i128()? as f64
        };
        (numerator / total as f64).to_string()
    };
    Ok(format!("avg({column})\n{rendered}\n").into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusthouse_query::parse_select;

    #[test]
    fn weights_shards_by_nonnull_rows_not_by_peer_count() {
        let shards = [
            (10, b"sum(amount)\tcount(amount)\n30\t10\n".to_vec()),
            (10, b"sum(amount)\tcount(amount)\n100\t1\n".to_vec()),
        ];
        assert_eq!(
            merge_global_avg(&shards, "amount", "UInt64").unwrap(),
            b"avg(amount)\n11.818181818181818\n"
        );
    }

    #[test]
    fn signed_average_and_empty_peers_keep_null_semantics() {
        let shards = [
            (10, b"sum(x)\tcount(x)\n-8\t2\n".to_vec()),
            (10, b"sum(x)\tcount(x)\nNULL\t0\n".to_vec()),
            (10, b"sum(x)\tcount(x)\n2\t1\n".to_vec()),
        ];
        assert_eq!(
            merge_global_avg(&shards, "x", "Nullable(Int32)").unwrap(),
            b"avg(x)\n-2\n"
        );
        assert_eq!(
            merge_global_avg(
                &[(10, b"sum(x)\tcount(x)\nNULL\t0\n".to_vec())],
                "x", "Int64",
            ).unwrap(),
            b"avg(x)\nNULL\n"
        );
    }

    #[test]
    fn rejects_inconsistent_partial_sums_and_counts() {
        for row in ["NULL\t2", "5\t0", "5\t-1", "5\t01", "5\t1\t9"] {
            assert_eq!(
                merge_global_avg(
                    &[(10, format!("sum(x)\tcount(x)\n{row}\n").into_bytes())],
                    "x", "Int64"
                ),
                Err(MergeError::InvalidSum),
            );
        }
        assert_eq!(
            merge_global_avg(
                &[(10, b"sum(x)\tcount(x)\n5\t1\n".to_vec())], "x", "Int128"
            ),
            Err(MergeError::InvalidSum)
        );
    }

    #[test]
    fn rewrites_average_with_where_to_exact_partial_states() {
        let sql = "SELECT avg(x) FROM events WHERE x >= -4";
        let parsed = parse_select(sql).unwrap();
        assert_eq!(
            super::super::avg_shard_sql(sql, &parsed).as_deref(),
            Some("SELECT sum(x), count(x) FROM events WHERE x >= -4"),
        );
    }
}
