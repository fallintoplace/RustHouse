//! Globally correct, constant-memory AVG reduction over disjoint analytical shards.
//! Never average shard means: a partition with one row must not weigh as much as
//! a partition with millions of rows.

use super::MergeError;

/// Each shard computes sum(value), count(value) in a single read.
/// Exact integer sums avoid truncation before the final Float64 division.
pub(super) fn merge_integer_averages(
    shards: &[(usize, Vec<u8>)],
    kind: &str,
    column: &str,
    alias: Option<&str>,
) -> Result<Vec<u8>, MergeError> {
    if shards.is_empty() {
        return Err(MergeError::InvalidAverage);
    }
    let header = format!("sum({column})\tcount({column})\n");
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let mut signed_sum = 0i128;
    let mut unsigned_sum = 0u128;
    let mut total_count = 0u64;
    for (_, body) in shards {
        let body = body
            .strip_prefix(header.as_bytes())
            .ok_or(MergeError::InvalidAverage)?;
        let line = body.strip_suffix(b"\n").ok_or(MergeError::InvalidAverage)?;
        if line.contains(&b'\n') {
            return Err(MergeError::InvalidAverage);
        }
        let delimiter = line
            .iter()
            .position(|&byte| byte == b'\t')
            .ok_or(MergeError::InvalidAverage)?;
        let (sum, count) = (&line[..delimiter], &line[delimiter + 1..]);
        if count.is_empty() || !count.iter().all(u8::is_ascii_digit) {
            return Err(MergeError::InvalidAverage);
        }
        let count = std::str::from_utf8(count)
            .map_err(|_| MergeError::InvalidAverage)?
            .parse::<u64>()
            .map_err(|_| MergeError::CountOverflow)?;
        if (sum == b"NULL") != (count == 0) {
            return Err(MergeError::InvalidAverage);
        }
        total_count = total_count
            .checked_add(count)
            .ok_or(MergeError::CountOverflow)?;
        if count == 0 {
            continue;
        }
        let value = std::str::from_utf8(sum).map_err(|_| MergeError::InvalidAverage)?;
        if unsigned {
            unsigned_sum = unsigned_sum
                .checked_add(value.parse::<u128>().map_err(|_| MergeError::InvalidAverage)?)
                .ok_or(MergeError::SumOverflow)?;
        } else {
            signed_sum = signed_sum
                .checked_add(value.parse::<i128>().map_err(|_| MergeError::InvalidAverage)?)
                .ok_or(MergeError::SumOverflow)?;
        }
    }
    let average = if total_count == 0 {
        "NULL".to_owned()
    } else {
        let sum = if unsigned {
            unsigned_sum as f64
        } else {
            signed_sum as f64
        };
        (sum / total_count as f64).to_string()
    };
    let name = alias.map_or_else(|| format!("avg({column})"), str::to_owned);
    Ok(format!("{name}\n{average}\n").into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shard(data: &str) -> (usize, Vec<u8>) {
        (1000, data.as_bytes().to_vec())
    }

    #[test]
    fn weighted_avg_keeps_unequal_shards_correct() {
        let shards = [
            shard("sum(amount)\tcount(amount)\n200\t2\n"),
            shard("sum(amount)\tcount(amount)\n50\t1\n"),
            shard("sum(amount)\tcount(amount)\nNULL\t0\n"),
        ];
        assert_eq!(
            merge_integer_averages(&shards, "UInt64", "amount", None).unwrap(),
            b"avg(amount)\n83.33333333333333\n"
        );
        assert_eq!(
            merge_integer_averages(
                &[shard("sum(amount)\tcount(amount)\nNULL\t0\n")],
                "Nullable(Int64)",
                "amount",
                Some("mean")
            ).unwrap(),
            b"mean\nNULL\n"
        );
        assert_eq!(
            merge_integer_averages(
                &[shard("sum(amount)\tcount(amount)\n-7\t2\n")],
                "Int64",
                "amount",
                None,
            ).unwrap(),
            b"avg(amount)\n-3.5\n"
        );
    }

    #[test]
    fn corrupt_or_overflowed_shard_state_rejected() {
        for data in [
            "sum(amount)\tcount(amount)\nNULL\t2\n",
            "sum(amount)\tcount(amount)\n5\t0\n",
            "sum(amount)\tcount(amount)\n1\t1\n2\t1\n",
            "sum(amount)\tcount(amount)\n1\tone\n",
            "sum(amount)\tcount(amount)\n1\t1",
            "sum(amount)\tcount(amount)\n1\t1\t2\n",
        ] {
            assert_eq!(
                merge_integer_averages(&[shard(data)], "Int64", "amount", None),
                Err(MergeError::InvalidAverage),
                "{data}"
            );
        }
        assert_eq!(
            merge_integer_averages(
                &[
                    shard(&format!("sum(amount)\tcount(amount)\n{}\t1\n", i128::MAX)),
                    shard("sum(amount)\tcount(amount)\n1\t1\n")
                ],
                "Int128",
                "amount",
                None
            ),
            Err(MergeError::SumOverflow)
        );
    }

    #[test]
    fn rewrite_preserves_predicates_and_rejects_global_limit() {
        let sql = "SELECT avg(amount) FROM events WHERE region = 'FROM east'";
        let parsed = crate::parse_select(sql).unwrap();
        assert_eq!(
            super::super::query_policy::avg_shard_sql(sql, &parsed).as_deref(),
            Some("SELECT sum(amount), count(amount) FROM events WHERE region = 'FROM east'")
        );
        let parsed = crate::parse_select("SELECT avg(amount) FROM events LIMIT 10").unwrap();
        assert!(!super::super::query_policy::is_safe_avg(&parsed));
    }
}
