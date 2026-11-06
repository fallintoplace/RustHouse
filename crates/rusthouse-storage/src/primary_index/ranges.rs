//! Keep primary-key interval pruning and physical index loading in distinct modules.

use super::*;

/// Conservative granule range for an ascending fixed-width index.
///
/// A mark stores the first key of a granule. The predecessor of the first
/// matching mark can also contain matching rows, so range selection must
/// retain it. Callers still apply the exact row predicate.
/// Decode a narrow physical key without sign loss or host-endian assumptions.
fn narrow_unsigned_mark_key(index: &FixedPrimaryIndex, mark: usize, field: usize) -> Option<u64> {
    match index.field(mark, field)? {
        [a] => Some(u64::from(*a)),
        [a, b] => Some(u64::from(u16::from_le_bytes([*a, *b]))),
        [a, b, c, d] => Some(u64::from(u32::from_le_bytes([*a, *b, *c, *d]))),
        _ => None,
    }
}

fn narrow_signed_mark_key(index: &FixedPrimaryIndex, mark: usize, field: usize) -> Option<i64> {
    match index.field(mark, field)? {
        [a] => Some(i64::from(i8::from_le_bytes([*a]))),
        [a, b] => Some(i64::from(i16::from_le_bytes([*a, *b]))),
        [a, b, c, d] => Some(i64::from(i32::from_le_bytes([*a, *b, *c, *d]))),
        _ => None,
    }
}

/// Conservatively select mark ranges for UInt8, UInt16, or UInt32 key fields.
/// Bounds use u64 so callers can express values beyond the physical type's
/// range without truncation or wraparound.
pub fn candidate_narrow_unsigned_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<u64>,
    upper: Bound<u64>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| narrow_unsigned_mark_key(index, mark, field),
        lower,
        upper,
    )
}

/// Select mark ranges for signed Int8, Int16, or Int32 primary-key fields.
pub fn candidate_narrow_signed_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<i64>,
    upper: Bound<i64>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| narrow_signed_mark_key(index, mark, field),
        lower,
        upper,
    )
}

/// Merge disjunctive intervals for UInt8/16/32 primary-key fields.
pub fn candidate_narrow_unsigned_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<u64>, Bound<u64>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| narrow_unsigned_mark_key(index, mark, field),
        intervals,
    )
}

/// Merge disjunctive intervals for Int8/16/32 primary-key fields.
pub fn candidate_narrow_signed_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<i64>, Bound<i64>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| narrow_signed_mark_key(index, mark, field),
        intervals,
    )
}

/// Returns a conservative mark range for an ascending UInt64 key.
/// Invalid or unsorted bounded indexes return None. Fully unbounded scans
/// return every granule without reading or validating individual keys.
pub fn candidate_u64_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<u64>,
    upper: Bound<u64>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| u64_mark_key(index, mark, field),
        lower,
        upper,
    )
}

/// Returns a conservative mark range for an ascending Int64 key.
///
/// Signed comparisons use the decoded Int64 values rather than their raw
/// little-endian byte order. This is only a planning primitive: callers must
/// still evaluate the predicate for candidate rows.
pub fn candidate_i64_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<i64>,
    upper: Bound<i64>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| i64_mark_key(index, mark, field),
        lower,
        upper,
    )
}

fn u64_pair_mark_key(
    index: &FixedPrimaryIndex,
    mark: usize,
    first: usize,
    second: usize,
) -> Option<(u64, u64)> {
    Some((
        u64_mark_key(index, mark, first)?,
        u64_mark_key(index, mark, second)?,
    ))
}

fn i64_pair_mark_key(
    index: &FixedPrimaryIndex,
    mark: usize,
    first: usize,
    second: usize,
) -> Option<(i64, i64)> {
    Some((
        i64_mark_key(index, mark, first)?,
        i64_mark_key(index, mark, second)?,
    ))
}

/// Plan conservative lexicographic granules for two UInt64 primary-key fields.
/// The second key can reset when the first key changes; validating either
/// column independently would incorrectly disable composite index pruning.
pub fn candidate_u64_pair_mark_range(
    index: &FixedPrimaryIndex,
    first: usize,
    second: usize,
    lower: Bound<(u64, u64)>,
    upper: Bound<(u64, u64)>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| u64_pair_mark_key(index, mark, first, second),
        lower,
        upper,
    )
}

/// Plan conservative lexicographic granules for two signed Int64 key fields.
pub fn candidate_i64_pair_mark_range(
    index: &FixedPrimaryIndex,
    first: usize,
    second: usize,
    lower: Bound<(i64, i64)>,
    upper: Bound<(i64, i64)>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| i64_pair_mark_key(index, mark, first, second),
        lower,
        upper,
    )
}

/// A disjunctive interval of two unsigned primary-key fields.
pub type U64PairInterval = (Bound<(u64, u64)>, Bound<(u64, u64)>);

/// Union disjunctive lexicographic UInt64 tuple intervals.
pub fn candidate_u64_pair_mark_ranges(
    index: &FixedPrimaryIndex,
    first: usize,
    second: usize,
    intervals: &[U64PairInterval],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| u64_pair_mark_key(index, mark, first, second),
        intervals,
    )
}

/// A disjunctive interval of two signed primary-key fields.
pub type I64PairInterval = (Bound<(i64, i64)>, Bound<(i64, i64)>);

/// Union disjunctive lexicographic Int64 tuple intervals.
pub fn candidate_i64_pair_mark_ranges(
    index: &FixedPrimaryIndex,
    first: usize,
    second: usize,
    intervals: &[I64PairInterval],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| i64_pair_mark_key(index, mark, first, second),
        intervals,
    )
}

/// Plan and coalesce candidate granules for UInt64 IN and OR predicates.
pub fn candidate_u64_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<u64>, Bound<u64>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| u64_mark_key(index, mark, field),
        intervals,
    )
}

/// Plan and coalesce candidate granules for signed Int64 predicates.
pub fn candidate_i64_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<i64>, Bound<i64>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| i64_mark_key(index, mark, field),
        intervals,
    )
}

/// Conservatively prune a sorted UInt128 primary key, retaining a
/// predecessor granule so row-level predicates remain exact.
pub fn candidate_u128_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<u128>,
    upper: Bound<u128>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| u128_mark_key(index, mark, field),
        lower,
        upper,
    )
}

/// Conservatively prune a signed Int128 primary key.
pub fn candidate_i128_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<i128>,
    upper: Bound<i128>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| i128_mark_key(index, mark, field),
        lower,
        upper,
    )
}

/// Coalesce disjunctive UInt128 primary key intervals into mark ranges.
pub fn candidate_u128_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<u128>, Bound<u128>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| u128_mark_key(index, mark, field),
        intervals,
    )
}

/// Coalesce disjunctive Int128 primary key intervals into mark ranges.
pub fn candidate_i128_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<i128>, Bound<i128>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| i128_mark_key(index, mark, field),
        intervals,
    )
}
