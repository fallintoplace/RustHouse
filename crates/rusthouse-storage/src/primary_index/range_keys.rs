use super::*;

fn monotonic_mark_keys<T: Ord + Copy>(len: usize, key_at: &impl Fn(usize) -> Option<T>) -> bool {
    let mut previous = None;
    for mark in 0..len {
        let Some(key) = key_at(mark) else {
            return false;
        };
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
    // Full-range scans cannot accidentally omit a matching granule.
    // Avoid validating every key when no index boundary will be applied.
    if matches!(lower, Bound::Unbounded) && matches!(upper, Bound::Unbounded) {
        return Some(0..len);
    }
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
    // A disjunct that includes every value dominates all narrower intervals.
    if intervals.iter().any(|(lower, upper)| {
        matches!(lower, Bound::Unbounded) && matches!(upper, Bound::Unbounded)
    }) {
        return Some(if len == 0 {
            Vec::new()
        } else {
            std::iter::once(0..len).collect()
        });
    }
    if !monotonic_mark_keys(len, &key_at) {
        return None;
    }
    if let [(lower, upper)] = intervals {
        let range = candidate_mark_range_sorted(len, &key_at, *lower, *upper);
        return Some(if range.is_empty() {
            Vec::new()
        } else {
            vec![range]
        });
    }
    let mut ranges: Vec<_> = intervals
        .iter()
        .map(|(lower, upper)| candidate_mark_range_sorted(len, &key_at, *lower, *upper))
        .filter(|range| !range.is_empty())
        .collect();
    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
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

fn key_bytes(index: &FixedPrimaryIndex, mark: usize, field: usize) -> Option<[u8; 8]> {
    index.field(mark, field)?.try_into().ok()
}

pub(super) fn u64_mark_key(index: &FixedPrimaryIndex, mark: usize, field: usize) -> Option<u64> {
    Some(u64::from_le_bytes(key_bytes(index, mark, field)?))
}

pub(super) fn i64_mark_key(index: &FixedPrimaryIndex, mark: usize, field: usize) -> Option<i64> {
    Some(i64::from_le_bytes(key_bytes(index, mark, field)?))
}

#[cfg(test)]
mod unbounded_range_tests {
    use super::*;

    #[test]
    fn full_scan_needs_no_primary_index_decoding() {
        let complete = candidate_mark_range::<u64>(
            17,
            |_| panic!("unbounded scan must not inspect mark keys"),
            Bound::Unbounded,
            Bound::Unbounded,
        );
        assert_eq!(complete, Some(0..17));

        let intervals = [
            (Bound::Included(4u64), Bound::Included(7u64)),
            (Bound::Unbounded, Bound::Unbounded),
        ];
        let merged = candidate_mark_ranges(
            17,
            |_| panic!("full-range OR must not inspect mark keys"),
            &intervals,
        );
        assert_eq!(merged, Some(std::iter::once(0..17).collect::<Vec<_>>()));
    }
}
