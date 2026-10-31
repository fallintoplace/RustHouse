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
) -> Result<Option<(IntegerKey, usize)>, MergeError> {
    if start == data.len() {
        return Ok(None);
    }
    let newline = data[start..].iter()
        .position(|&byte| byte == b'\n')
        .ok_or(MergeError::InvalidSortedScan)?;
    let end = start + newline + 1;
    let value = &data[start..end - 1];
    if value.contains(&b'\t') || value == b"NULL" {
        return Err(MergeError::InvalidSortedScan);
    }
    Ok(Some((parse_integer_sort_key(value, kind)?, end)))
}

/// Validate the full sorted shard streams before returning any rows.
/// Retain only one integer key and byte cursor per peer, rather than
/// materializing every shard row at the coordinator.
pub(super) fn merge_sorted_integer_page(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    kind: &str,
    descending: bool,
    offset: usize,
    limit: usize,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    let header = format!("{column}\n");
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
        while let Some((key, next)) = next_row(data, cursor, kind)? {
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
    while let Some(cursor) = heap.pop() {
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
            break;
        }
        if let Some((key, next_position)) = next_row(body, end, kind)? {
            heap.push(cursor.advance(key, next_position));
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
