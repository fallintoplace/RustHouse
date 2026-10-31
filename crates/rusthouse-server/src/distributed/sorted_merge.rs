//! Globally correct, bounded k-way merge of sorted integer shard prefixes.
use super::{MergeError, MAX_RESPONSE_BYTES};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IntegerKey {
    Signed(i128),
    Unsigned(u128),
}

#[derive(Clone, Copy)]
enum IntegerType {
    Signed { min: i128, max: i128 },
    Unsigned { max: u128 },
}

impl IntegerType {
    fn from_schema(kind: &str) -> Result<Self, MergeError> {
        let kind = kind.strip_prefix("Nullable(")
            .and_then(|inner| inner.strip_suffix(')'))
            .unwrap_or(kind);
        let (unsigned, bits) = match kind {
            "UInt8" => (true, 8), "UInt16" => (true, 16),
            "UInt32" => (true, 32), "UInt64" => (true, 64),
            "UInt128" => (true, 128),
            "Int8" => (false, 8), "Int16" => (false, 16),
            "Int32" => (false, 32), "Int64" => (false, 64),
            "Int128" => (false, 128),
            _ => return Err(MergeError::InvalidSortedScan),
        };
        Ok(if unsigned {
            Self::Unsigned {
                max: if bits == 128 { u128::MAX } else { (1u128 << bits) - 1 },
            }
        } else {
            Self::Signed {
                min: if bits == 128 { i128::MIN } else { -(1i128 << (bits - 1)) },
                max: if bits == 128 { i128::MAX } else { (1i128 << (bits - 1)) - 1 },
            }
        })
    }
}

// Parse network TSV digits directly. UTF-8 validation and FromStr perform
// redundant work for a grammar limited to canonical ASCII decimal integers.
fn parse_unsigned_ascii(bytes: &[u8]) -> Option<u128> {
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0u128;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u128::from(byte - b'0'))?;
    }
    Some(value)
}

fn parse_signed_ascii(bytes: &[u8]) -> Option<i128> {
    let negative = bytes.first() == Some(&b'-');
    let digits = if negative { &bytes[1..] } else { bytes };
    if digits.is_empty() {
        return None;
    }
    let mut value = 0i128;
    for &byte in digits {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?;
        let digit = i128::from(byte - b'0');
        value = if negative {
            value.checked_sub(digit)?
        } else {
            value.checked_add(digit)?
        };
    }
    Some(value)
}

fn parse_integer_sort_key(bytes: &[u8], kind: IntegerType) -> Result<IntegerKey, MergeError> {
    if bytes.starts_with(b"+")
        || (bytes.len() > 1 && bytes[0] == b'0')
        || bytes.starts_with(b"-0")
    {
        return Err(MergeError::InvalidSortedScan);
    }
    match kind {
        IntegerType::Unsigned { max } => {
            let number = parse_unsigned_ascii(bytes).ok_or(MergeError::InvalidSortedScan)?;
            (number <= max).then_some(IntegerKey::Unsigned(number))
                .ok_or(MergeError::InvalidSortedScan)
        }
        IntegerType::Signed { min, max } => {
            let number = parse_signed_ascii(bytes).ok_or(MergeError::InvalidSortedScan)?;
            (min <= number && number <= max).then_some(IntegerKey::Signed(number))
                .ok_or(MergeError::InvalidSortedScan)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cursor {
    key: IntegerKey,
    shard: usize,
    position: usize,
    next_position: usize,
    descending: bool,
}
impl Cursor {
    fn advance(self, key: IntegerKey, next_position: usize) -> Self {
        Self {
            key,
            position: self.next_position,
            next_position,
            ..self
        }
    }
}

impl Ord for Cursor {
    fn cmp(&self, other: &Self) -> Ordering {
        let cmp = self.key.cmp(&other.key);
        (if self.descending { cmp } else { cmp.reverse() })
            .then_with(|| other.shard.cmp(&self.shard))
            .then_with(|| other.position.cmp(&self.position))
    }
}
impl PartialOrd for Cursor {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

/// Locate and parse only the next wire row. Positions are byte offsets in
/// the immutable shard body, not offsets into a materialized row array.
fn next_row(
    data: &[u8],
    start: usize,
    kind: IntegerType,
    column_count: usize,
) -> Result<Option<(IntegerKey, usize)>, MergeError> {
    if start == data.len() {
        return Ok(None);
    }
    let newline = data[start..].iter()
        .position(|&byte| byte == b'\n')
        .ok_or(MergeError::InvalidSortedScan)?;
    let end = start + newline + 1;
    let row = &data[start..end - 1];
    // Only the first projected column is the sort key. The remaining fields
    // are opaque TSV values and are returned together with the original row.
    // Strict column counts prevent partial or injected peer rows.
    let mut fields = row.split(|&byte| byte == b'\t');
    let key = fields.next().ok_or(MergeError::InvalidSortedScan)?;
    // Count fields while validating them. The previous two passes scanned
    // every projected byte once to count tabs, then again to decode fields.
    // Reject excess fields immediately and missing fields at the end.
    let mut fields_seen = 1usize;
    for field in fields {
        fields_seen += 1;
        if fields_seen > column_count {
            return Err(MergeError::InvalidSortedScan);
        }
        // Even rows outside LIMIT/OFFSET must have valid TSV escaping.
        if field != br"\N" {
            rusthouse_format::validate_tsv_field(field)
                .map_err(|_| MergeError::InvalidSortedScan)?;
        }
    }
    if fields_seen != column_count {
        return Err(MergeError::InvalidSortedScan);
    }
    if key == b"NULL" {
        return Err(MergeError::InvalidSortedScan);
    }
    Ok(Some((parse_integer_sort_key(key, kind)?, end)))
}

// The coordinator already checked every field, row width and integer key
// during the complete input pass. Merging that immutable body only needs
// the key and next row offset, not another scan of every TSV escape.
fn next_validated_row(
    data: &[u8],
    start: usize,
    kind: IntegerType,
) -> Result<Option<(IntegerKey, usize)>, MergeError> {
    if start == data.len() {
        return Ok(None);
    }
    let rest = data.get(start..).ok_or(MergeError::InvalidSortedScan)?;
    let line_len = rest
        .iter()
        .position(|&byte| byte == b'\n')
        .ok_or(MergeError::InvalidSortedScan)?;
    let row = &rest[..line_len];
    let key_end = row.iter().position(|&byte| byte == b'\t').unwrap_or(row.len());
    let key = parse_integer_sort_key(&row[..key_end], kind)?;
    Ok(Some((key, start + line_len + 1)))
}

/// Validate the full sorted shard streams before returning any rows.
/// Retain only one integer key and byte cursor per peer, rather than
/// materializing every shard row at the coordinator.
#[cfg(test)]
pub(super) fn merge_sorted_integer_page(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    kind: &str,
    descending: bool,
    offset: usize,
    limit: usize,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    merge_sorted_integer_rows_page(
        shards, &[column], kind, descending, offset, limit, max_rows,
    )
}

/// Merge sorted shard prefixes while preserving complete projected rows.
/// The first column is an exact, non-nullable integer ORDER BY key.
pub(super) fn merge_sorted_integer_rows_page(
    shards: &[(usize, Vec<u8>)],
    columns: &[&str],
    kind: &str,
    descending: bool,
    offset: usize,
    limit: usize,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    if !(1..=16).contains(&columns.len()) {
        return Err(MergeError::InvalidSortedScan);
    }
    let header = format!("{}\n", columns.join("\t"));
    let header_len = header.len();
    let kind = IntegerType::from_schema(kind)?;
    let prefix = offset.checked_add(limit).ok_or(MergeError::CoordinatorLimit)?;
    if shards.is_empty() {
        return Err(MergeError::InvalidSortedScan);
    }
    let mut first_keys = Vec::with_capacity(shards.len());
    for (cap, body) in shards {
        if *cap <= prefix || !body.ends_with(b"\n") || !body.starts_with(header.as_bytes()) {
            return Err(MergeError::InvalidSortedScan);
        }
        let data = &body[header_len..];
        let mut cursor = 0usize;
        let mut rows = 0usize;
        let mut previous = None;
        let mut first = None;
        while let Some((key, next)) = next_row(data, cursor, kind, columns.len())? {
            if previous.is_some_and(|prior| if descending { prior < key } else { prior > key }) {
                return Err(MergeError::InvalidSortedScan);
            }
            if first.is_none() {
                first = Some((key, next));
            }
            previous = Some(key);
            rows += 1;
            if rows > prefix || rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            cursor = next;
        }
        first_keys.push(first);
    }
    let mut heap = BinaryHeap::new();
    for (shard, first) in first_keys.into_iter().enumerate() {
        if let Some((key, next_position)) = first {
            heap.push(Cursor {
                key, shard, position: 0, next_position, descending,
            });
        }
    }
    let mut output = header.into_bytes();
    let mut seen = 0usize;
    let mut returned = 0usize;
    // A shard can supply a long consecutive run before another shard wins
    // the global ordering. Keep its cursor outside the heap until it loses
    // priority, avoiding a heap pop/push for every row of a dominant run.
    'merge: while let Some(mut cursor) = heap.pop() {
        loop {
            let body = &shards[cursor.shard].1[header_len..];
            // The complete peer stream was already validated before heap merging.
            let end = cursor.next_position;
            if seen >= offset && returned < limit {
                let row = &body[cursor.position..end];
                if returned >= max_rows || output.len().checked_add(row.len())
                    .is_none_or(|size| size > MAX_RESPONSE_BYTES)
                {
                    return Err(MergeError::CoordinatorLimit);
                }
                output.extend_from_slice(row);
                returned += 1;
            }
            seen += 1;
            if returned >= limit {
                break 'merge;
            }
            let Some((key, next_position)) = next_validated_row(body, end, kind)? else {
                break;
            };
            let advanced = cursor.advance(key, next_position);
            if heap.peek().is_some_and(|other| other > &advanced) {
                heap.push(advanced);
                break;
            }
            cursor = advanced;
        }
    }
    Ok(output)
}


#[cfg(test)]
mod ascii_integer_merge_tests {
    use super::*;

    #[test]
    fn exact_ascii_parser_handles_bounds_and_canonical_wire_encoding() {
        for number in [b"0".as_slice(), b"1", b"18446744073709551615",
            b"340282366920938463463374607431768211455"] {
            assert_eq!(parse_unsigned_ascii(number), std::str::from_utf8(number).unwrap().parse::<u128>().ok());
        }
        for number in [b"0".as_slice(), b"-1", b"170141183460469231731687303715884105727",
            b"-170141183460469231731687303715884105728"] {
            assert_eq!(parse_signed_ascii(number), std::str::from_utf8(number).unwrap().parse::<i128>().ok());
        }
        for invalid in [b"".as_slice(), b"-", b"1x", b" 1", b"1 ", b"\xff",
            b"340282366920938463463374607431768211456"] {
            assert!(parse_unsigned_ascii(invalid).is_none());
        }
        for invalid in [b"".as_slice(), b"-", b"1x", b"1 ", b"\xff",
            b"170141183460469231731687303715884105728",
            b"-170141183460469231731687303715884105729"] {
            assert!(parse_signed_ascii(invalid).is_none());
        }
        let signed = IntegerType::Signed { min: i128::MIN, max: i128::MAX };
        for invalid in [b"+1".as_slice(), b"00", b"01", b"-0", b"-01"] {
            assert!(parse_integer_sort_key(invalid, signed).is_err());
        }
        assert_eq!(parse_integer_sort_key(b"-170141183460469231731687303715884105728", signed),
            Ok(IntegerKey::Signed(i128::MIN)));
    }
}

#[cfg(test)]
#[path = "sorted_merge/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sorted_merge/large_sorted_merge_tests.rs"]
mod large_sorted_merge_tests;

#[cfg(test)]
#[path = "sorted_merge/cursor_boundary_regression_tests.rs"]
mod cursor_boundary_regression_tests;

#[cfg(test)]
mod schema_width_regression_tests {
    use super::*;

    #[test]
    fn rejects_peer_values_outside_declared_integer_width() {
        for (kind, value) in [
            ("UInt8", "256"), ("UInt16", "65536"),
            ("UInt32", "4294967296"), ("UInt64", "18446744073709551616"),
            ("Int8", "-129"), ("Int8", "128"), ("Int16", "32768"),
            ("Int32", "-2147483649"), ("Int64", "9223372036854775808"),
            ("Nullable(UInt64)", "18446744073709551616"),
            ("UInt128", "-1"),
        ] {
            let wire = format!("id\n{value}\n").into_bytes();
            assert_eq!(
                merge_sorted_integer_page(&[(4, wire)], "id", kind, false, 0, 1, 1),
                Err(MergeError::InvalidSortedScan), "{kind}: {value}"
            );
        }
    }

    #[test]
    fn accepts_bounds_and_unsigned_nullable_types() {
        for (kind, value) in [
            ("Int8", "-128"), ("Int8", "127"), ("UInt8", "255"),
            ("Int16", "-32768"), ("UInt16", "65535"),
            ("Int64", "-9223372036854775808"),
            ("UInt64", "18446744073709551615"),
            ("Int128", "-170141183460469231731687303715884105728"),
            ("UInt128", "340282366920938463463374607431768211455"),
            ("Nullable(UInt128)", "340282366920938463463374607431768211455"),
        ] {
            let wire = format!("id\n{value}\n").into_bytes();
            assert!(merge_sorted_integer_page(&[(4, wire)], "id", kind, false, 0, 1, 1).is_ok(),
                "{kind}: {value}");
        }
        assert_eq!(
            merge_sorted_integer_page(&[(4, b"id\n1\n".to_vec())], "id", "Unknown", false, 0, 1, 1),
            Err(MergeError::InvalidSortedScan)
        );
    }
}

#[cfg(test)]
mod multicolumn_sorted_merge_tests {
    use super::*;

    #[test]
    fn merges_full_rows_and_preserves_stable_ties_across_peers() {
        let shards = [
            (16, b"id\tname\n1\tb\n4\tz\n7\tlast\n".to_vec()),
            (16, b"id\tname\n1\ta\n2\tc\\td\n8\tn\n".to_vec()),
        ];
        let page = merge_sorted_integer_rows_page(
            &shards, &["id", "name"], "UInt64", false, 1, 3, 3,
        ).unwrap();
        assert_eq!(page, b"id\tname\n1\ta\n2\tc\\td\n4\tz\n");
    }

    #[test]
    fn checks_all_shard_fields_even_after_page_is_full() {
        for invalid in [
            b"id\tname\n1\n".as_slice(),
            b"id\tname\n1\tok\textra\n".as_slice(),
            b"id\tname\n1\tok\n0\tout-of-order\n".as_slice(),
        ] {
            assert_eq!(
                merge_sorted_integer_rows_page(
                    &[(16, invalid.to_vec())], &["id", "name"], "UInt64", false, 0, 1, 1,
                ),
                Err(MergeError::InvalidSortedScan),
            );
        }
        assert_eq!(
            merge_sorted_integer_rows_page(
                &[(16, b"id\tname\n".to_vec())], &[], "UInt64", false, 0, 1, 1,
            ),
            Err(MergeError::InvalidSortedScan),
        );
    }

    #[test]
    fn signed_descending_multirow_merge_supports_pagination() {
        let shards = [
            (10, b"id\tlabel\n9\tfirst\n2\tsecond\n".to_vec()),
            (10, b"id\tlabel\n8\tother\n-4\tnegative\n".to_vec()),
        ];
        assert_eq!(
            merge_sorted_integer_rows_page(
                &shards, &["id", "label"], "Int64", true, 1, 2, 2,
            ).unwrap(),
            b"id\tlabel\n8\tother\n2\tsecond\n",
        );
    }
}

#[cfg(test)]
mod sorted_shard_escaped_fields_tests {
    use super::*;

    #[test]
    fn validates_all_projected_fields_before_returning_a_page() {
        for corrupt in [
            b"id\tname\n1\tgood\n2\tbad\\q\n".as_slice(),
            b"id\tname\n1\tgood\n2\tbad\\x0\n".as_slice(),
            b"id\tname\n1\tgood\n2\tbad\\xGG\n".as_slice(),
            b"id\tname\n1\tgood\n2\tbad\\\n".as_slice(),
        ] {
            assert_eq!(
                merge_sorted_integer_rows_page(
                    &[(16, corrupt.to_vec())], &["id", "name"],
                    "UInt64", false, 0, 1, 1,
                ),
                Err(MergeError::InvalidSortedScan),
            );
        }
        let valid = [(16, b"id\tname\n1\tvalid\\tvalue\n2\t\\N\n3\t\\x41\n".to_vec())];
        assert_eq!(
            merge_sorted_integer_rows_page(
                &valid, &["id", "name"], "UInt64", false, 1, 2, 2,
            ).unwrap(),
            b"id\tname\n2\t\\N\n3\t\\x41\n",
        );
    }
}

#[cfg(test)]
mod dominant_shard_run_tests {
    use super::*;

    #[test]
    fn long_dominant_runs_match_global_stable_merge_with_pagination() {
        let original: [Vec<i64>; 4] = [
            (0..48).map(|i| i * 2).collect(),
            (30..78).collect(),
            vec![0, 0, 1, 3, 3, 3, 15, 40, 44, 91, 92, 93, 94],
            vec![4, 4, 4, 7, 29, 49, 55, 60, 80, 99],
        ];
        for descending in [false, true] {
            let ordered = original.iter().map(|values| {
                let mut values = values.clone();
                if descending { values.reverse(); }
                values
            }).collect::<Vec<_>>();
            let mut expected_all = ordered.iter().enumerate()
                .flat_map(|(shard, values)| values.iter().enumerate()
                    .map(move |(row, &value)| (value, shard, row)))
                .collect::<Vec<_>>();
            expected_all.sort_by(|a, b| {
                let cmp = a.0.cmp(&b.0);
                (if descending { cmp.reverse() } else { cmp })
                    .then_with(|| a.1.cmp(&b.1))
                    .then_with(|| a.2.cmp(&b.2))
            });
            for offset in [0usize, 1, 7, 33, 50] {
                for limit in [1usize, 2, 5, 17, 31] {
                    let prefix = offset + limit;
                    let shards = ordered.iter().map(|values| {
                        let mut body = b"id\tlabel\n".to_vec();
                        for &value in values.iter().take(prefix) {
                            body.extend_from_slice(format!("{value}\tshard_row\n").as_bytes());
                        }
                        (prefix + 1, body)
                    }).collect::<Vec<_>>();
                    let actual = merge_sorted_integer_rows_page(
                        &shards, &["id", "label"], "Int64", descending, offset, limit, limit,
                    ).unwrap();
                    let mut expected = b"id\tlabel\n".to_vec();
                    for &(value, _, _) in expected_all.iter().skip(offset).take(limit) {
                        expected.extend_from_slice(format!("{value}\tshard_row\n").as_bytes());
                    }
                    assert_eq!(actual, expected,
                        "descending={descending} offset={offset} limit={limit}");
                }
            }
        }
    }

    #[test]
    fn early_page_limit_never_reads_past_validated_rows() {
        let shards = [(3, b"id\n1\n2\n".to_vec()), (3, b"id\n20\n21\n".to_vec())];
        assert_eq!(merge_sorted_integer_rows_page(
            &shards, &["id"], "Int64", false, 0, 1, 1,
        ).unwrap(), b"id\n1\n");
    }
}
#[cfg(test)]
mod one_pass_tsv_field_tests {
    use super::*;

    #[test]
    fn full_width_rows_and_page_boundaries_stay_exact() {
        let columns = (0..16).map(|i| format!("field_{i}")).collect::<Vec<_>>();
        let names = columns.iter().map(String::as_str).collect::<Vec<_>>();
        let mut body = format!("{}\n", columns.join("\t")).into_bytes();
        let first = {
            let mut values = vec!["1".to_owned()];
            values.extend((1..16).map(|index| format!("col_{index}")));
            format!("{}\n", values.join("\t"))
        };
        let second = {
            let mut values = vec!["2".to_owned()];
            values.extend((1..16).map(|_| "escaped\\tvalue".to_owned()));
            format!("{}\n", values.join("\t"))
        };
        body.extend_from_slice(first.as_bytes());
        body.extend_from_slice(second.as_bytes());
        let shard = [(3usize, body.clone())];
        assert_eq!(
            merge_sorted_integer_rows_page(&shard, &names, "UInt32", false, 1, 1, 1).unwrap(),
            [format!("{}\n", columns.join("\t")).as_bytes(), second.as_bytes()].concat()
        );

        let mut short = body.clone();
        let cut = short.windows(2).position(|w| w == b"\n2").unwrap() + 1;
        short.truncate(cut);
        short.extend_from_slice(b"2\tmissing\n");
        assert_eq!(
            merge_sorted_integer_rows_page(&[(3, short)], &names, "UInt32", false, 0, 1, 1),
            Err(MergeError::InvalidSortedScan)
        );
    }
}

#[cfg(test)]
mod validated_merge_cursor_tests {
    use super::*;

    #[test]
    fn fast_cursor_matches_complete_validator_over_many_wide_rows() {
        let kind = IntegerType::from_schema("Int64").unwrap();
        let mut wire = Vec::new();
        for value in -256..256 {
            wire.extend_from_slice(format!("{value}\tlong\\tescaped\tdata\\nline\n").as_bytes());
        }
        let mut cursor = 0;
        while cursor != wire.len() {
            let full = next_row(&wire, cursor, kind, 3).unwrap();
            let fast = next_validated_row(&wire, cursor, kind).unwrap();
            assert_eq!(full, fast);
            cursor = full.unwrap().1;
        }
        assert_eq!(next_validated_row(&wire, cursor, kind).unwrap(), None);
    }

    #[test]
    fn malformed_non_key_fields_still_fail_before_merging() {
        let shards = [(64usize, b"id\tpayload\n1\tgood\n2\tbad\\q\n".to_vec())];
        assert!(matches!(
            merge_sorted_integer_rows_page(&shards, &["id", "payload"], "Int64",
                false, 0, 1, 100),
            Err(MergeError::InvalidSortedScan)
        ));
    }
}
