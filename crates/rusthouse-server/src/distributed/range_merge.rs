//! Distributed RANGE derives the global extrema before subtraction.
//! Adding peer-local ranges would be mathematically incorrect.

use super::{MergeError, extrema_fits_declared_type};

pub(super) fn merge_global_range(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    kind: &str,
) -> Result<Vec<u8>, MergeError> {
    if shards.is_empty() {
        return Err(MergeError::InvalidExtremum);
    }
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let header = format!("min({column})\tmax({column})\n");
    let mut signed_min = None::<i128>;
    let mut signed_max = None::<i128>;
    let mut unsigned_min = None::<u128>;
    let mut unsigned_max = None::<u128>;
    for (cap, body) in shards {
        if *cap == 0 || !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidExtremum);
        }
        let values = &body[header.len()..body.len() - 1];
        let Some(tab) = values.iter().position(|&byte| byte == b'\t') else {
            return Err(MergeError::InvalidExtremum);
        };
        let lower = &values[..tab];
        let upper = &values[tab + 1..];
        if upper.contains(&b'\t') {
            return Err(MergeError::InvalidExtremum);
        }
        if lower == b"NULL" && upper == b"NULL" {
            continue;
        }
        if lower == b"NULL" || upper == b"NULL" {
            return Err(MergeError::InvalidExtremum);
        }
        let low = std::str::from_utf8(lower).map_err(|_| MergeError::InvalidExtremum)?;
        let high = std::str::from_utf8(upper).map_err(|_| MergeError::InvalidExtremum)?;
        if !extrema_fits_declared_type(kind, low) || !extrema_fits_declared_type(kind, high) {
            return Err(MergeError::InvalidExtremum);
        }
        if unsigned {
            let lo = low.parse::<u128>().map_err(|_| MergeError::InvalidExtremum)?;
            let hi = high.parse::<u128>().map_err(|_| MergeError::InvalidExtremum)?;
            if lo > hi {
                return Err(MergeError::InvalidExtremum);
            }
            unsigned_min = Some(unsigned_min.map_or(lo, |old| old.min(lo)));
            unsigned_max = Some(unsigned_max.map_or(hi, |old| old.max(hi)));
        } else {
            let lo = low.parse::<i128>().map_err(|_| MergeError::InvalidExtremum)?;
            let hi = high.parse::<i128>().map_err(|_| MergeError::InvalidExtremum)?;
            if lo > hi {
                return Err(MergeError::InvalidExtremum);
            }
            signed_min = Some(signed_min.map_or(lo, |old| old.min(lo)));
            signed_max = Some(signed_max.map_or(hi, |old| old.max(hi)));
        }
    }
    let rendered = if unsigned {
        match (unsigned_min, unsigned_max) {
            (Some(lo), Some(hi)) => (hi - lo).to_string(),
            (None, None) => "NULL".into(),
            _ => return Err(MergeError::InvalidExtremum),
        }
    } else {
        match (signed_min, signed_max) {
            (Some(lo), Some(hi)) => hi.abs_diff(lo).to_string(),
            (None, None) => "NULL".into(),
            _ => return Err(MergeError::InvalidExtremum),
        }
    };
    Ok(format!("range({column})\n{rendered}\n").into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusthouse_query::parse_select;

    #[test]
    fn full_signed_128_bit_range_across_shards_is_exact() {
        let shards = [
            (10, format!("min(amount)\tmax(amount)\n{}\n", i128::MAX).replace("\n", "\t").into_bytes()),
            (10, format!("min(amount)\tmax(amount)\n{}\t{}\n", i128::MIN, i128::MIN).into_bytes()),
            (10, format!("min(amount)\tmax(amount)\n{}\t{}\n", i128::MAX, i128::MAX).into_bytes()),
        ];
        let valid = &shards[1..];
        assert_eq!(
            merge_global_range(valid, "amount", "Int128").unwrap(),
            format!("range(amount)\n{}\n", u128::MAX).into_bytes()
        );
    }

    #[test]
    fn combines_unsigned_extrema_and_skips_empty_shards() {
        let shards = [
            (10, b"min(x)\tmax(x)\n5\t9\n".to_vec()),
            (10, b"min(x)\tmax(x)\nNULL\tNULL\n".to_vec()),
            (10, b"min(x)\tmax(x)\n2\t7\n".to_vec()),
        ];
        assert_eq!(
            merge_global_range(&shards, "x", "UInt64").unwrap(),
            b"range(x)\n7\n"
        );
    }

    #[test]
    fn mixed_nulls_and_inverted_ranges_are_corrupted_partial_states() {
        for body in [
            b"min(x)\tmax(x)\nNULL\t3\n".as_slice(),
            b"min(x)\tmax(x)\n8\t3\n",
            b"min(x)\tmax(x)\n1\t2\t3\n",
            b"min(x)\tmax(x)\n-1\t2\n",
        ] {
            assert_eq!(
                merge_global_range(&[(10, body.to_vec())], "x", "UInt8"),
                Err(MergeError::InvalidExtremum)
            );
        }
    }

    #[test]
    fn rewrite_keeps_original_filter_semantics() {
        let sql = "SELECT range(amount) FROM events WHERE amount > -5";
        let parsed = parse_select(sql).unwrap();
        assert_eq!(
            super::super::range_shard_sql(sql, &parsed).as_deref(),
            Some("SELECT min(amount), max(amount) FROM events WHERE amount > -5")
        );
    }
}
