use super::*;

fn monotonic_mark_keys<T: Ord + Copy>(
    len: usize,
    key_at: &impl Fn(usize) -> Option<T>,
) -> bool {
    let mut previous = None;
    for mark in 0..len {
        let Some(key) = key_at(mark) else { return false };
        if previous.is_some_and(|last| last > key) {
            return false;
        }
        previous = Some(key);
    }
    true
}

fn candidate_mark_range_sorted<T: Ord + Copy>(
    len: usize,
    key_at: &impl Fn(usize) -> Option<T>,
    lower: Bound<T>,
    upper: Bound<T>,
) -> Range<usize> {
    let partition_point = |target: T, inclusive: bool| {
        let mut low = 0;
        let mut high = len;
        while low < high {
            let mid = low + (high - low) / 2;
            let key = key_at(mid).expect("validated monotonic mark keys");
            if key < target || (inclusive && key == target) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    };
    let start = match lower {
        Bound::Included(value) => partition_point(value, false).saturating_sub(1),
        Bound::Excluded(value) => partition_point(value, true).saturating_sub(1),
        Bound::Unbounded => 0,
    };
    let end = match upper {
        Bound::Included(value) => partition_point(value, true),
        Bound::Excluded(value) => partition_point(value, false),
        Bound::Unbounded => len,
    };
    start.min(end)..end
}

pub(super) fn candidate_mark_range<T: Ord + Copy>(
    len: usize,
    key_at: impl Fn(usize) -> Option<T>,
    lower: Bound<T>,
    upper: Bound<T>,
) -> Option<Range<usize>> {
    monotonic_mark_keys(len, &key_at)
        .then(|| candidate_mark_range_sorted(len, &key_at, lower, upper))
}

/// Validate the entire sorted index only once when planning several
/// disjunctive candidate intervals. The union retains predecessor granules.
pub(super) fn candidate_mark_ranges<T: Ord + Copy>(
    len: usize,
    key_at: impl Fn(usize) -> Option<T>,
    intervals: &[(Bound<T>, Bound<T>)],
) -> Option<Vec<Range<usize>>> {
    if intervals.is_empty() {
        return Some(Vec::new());
    }
    if !monotonic_mark_keys(len, &key_at) {
        return None;
    }
    let mut ranges: Vec<_> = intervals.iter()
        .map(|(lower, upper)| candidate_mark_range_sorted(
            len, &key_at, lower.clone(), upper.clone()
        ))
        .filter(|range| !range.is_empty())
        .collect();
    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut() {
            if range.start <= previous.end {
                previous.end = previous.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }
    Some(merged)
}

pub(super) fn u64_mark_key(index: &FixedPrimaryIndex, mark: usize, field: usize) -> Option<u64> {
    let bytes: [u8; 8] = index.field(mark, field)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

pub(super) fn i64_mark_key(index: &FixedPrimaryIndex, mark: usize, field: usize) -> Option<i64> {
    let bytes: [u8; 8] = index.field(mark, field)?.try_into().ok()?;
    Some(i64::from_le_bytes(bytes))
}
