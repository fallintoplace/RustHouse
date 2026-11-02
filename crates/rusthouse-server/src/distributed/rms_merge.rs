//! Numerically stable global root-mean-square from weighted peer RMS states.
//! The weighted hypot reduction avoids squaring large floating partials.

use super::MergeError;

pub(super) fn merge_global_rms(
    shards: &[(usize, Vec<u8>)],
    column: &str,
) -> Result<Vec<u8>, MergeError> {
    let header = format!("rms({column})\tcount({column})\n");
    let mut root_mean_square = 0.0f64;
    let mut total_count = 0u128;
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
        let raw_rms = &row[..tab];
        let raw_count = &row[tab + 1..];
        if raw_count.contains(&b'\t') {
            return Err(MergeError::InvalidSum);
        }
        let count_text = std::str::from_utf8(raw_count).map_err(|_| MergeError::InvalidSum)?;
        let count = count_text.parse::<u128>().map_err(|_| MergeError::InvalidSum)?;
        if count.to_string() != count_text || (count == 0) != (raw_rms == b"NULL") {
            return Err(MergeError::InvalidSum);
        }
        if count == 0 {
            continue;
        }
        let value = std::str::from_utf8(raw_rms).map_err(|_| MergeError::InvalidSum)?
            .parse::<f64>()
            .map_err(|_| MergeError::InvalidSum)?;
        if !value.is_finite() || value < 0.0 {
            return Err(MergeError::InvalidSum);
        }
        let next_count = total_count.checked_add(count).ok_or(MergeError::CountOverflow)?;
        let previous_weight = (total_count as f64 / next_count as f64).sqrt();
        let incoming_weight = (count as f64 / next_count as f64).sqrt();
        root_mean_square = (root_mean_square * previous_weight).hypot(value * incoming_weight);
        total_count = next_count;
    }
    let rendered = if total_count == 0 {
        "NULL".to_owned()
    } else {
        root_mean_square.to_string()
    };
    Ok(format!("rms({column})\n{rendered}\n").into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusthouse_query::parse_select;

    #[test]
    fn weighted_rms_handles_different_shard_sizes() {
        let shards = [
            (10, b"rms(x)\tcount(x)\n3\t1\n".to_vec()),
            (10, b"rms(x)\tcount(x)\n4\t1\n".to_vec()),
        ];
        let response = merge_global_rms(&shards, "x").unwrap();
        let number = std::str::from_utf8(&response)
            .unwrap()
            .lines()
            .nth(1)
            .unwrap()
            .parse::<f64>()
            .unwrap();
        let expected = (3.0f64.hypot(4.0)) / 2.0f64.sqrt();
        assert!((number - expected).abs() < 1e-12);
    }

    #[test]
    fn empty_shards_do_not_turn_a_valid_rms_into_null() {
        let shards = [
            (10, b"rms(x)\tcount(x)\nNULL\t0\n".to_vec()),
            (10, b"rms(x)\tcount(x)\n5\t2\n".to_vec()),
        ];
        assert_eq!(
            merge_global_rms(&shards, "x").unwrap(),
            b"rms(x)\n5\n"
        );
        assert_eq!(
            merge_global_rms(
                &[(10, b"rms(x)\tcount(x)\nNULL\t0\n".to_vec())], "x",
            ).unwrap(),
            b"rms(x)\nNULL\n"
        );
    }

    #[test]
    fn rejects_malformed_nan_and_negative_rms_states() {
        for row in ["NaN\t2", "-1\t2", "inf\t2", "2\t0", "NULL\t1", "2\t01", "2\t1\t0"] {
            assert_eq!(
                merge_global_rms(
                    &[(10, format!("rms(x)\tcount(x)\n{row}\n").into_bytes())], "x"
                ),
                Err(MergeError::InvalidSum),
                "{row}"
            );
        }
    }

    #[test]
    fn rewriter_preserves_numeric_where_filters() {
        let sql = "SELECT rms(x) FROM events WHERE x < 100";
        let parsed = parse_select(sql).unwrap();
        assert_eq!(
            super::super::rms_shard_sql(sql, &parsed).as_deref(),
            Some("SELECT rms(x), count(x) FROM events WHERE x < 100")
        );
    }
}
