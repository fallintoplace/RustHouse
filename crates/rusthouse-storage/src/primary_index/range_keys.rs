use super::*;

pub(super) fn candidate_mark_range<T: Ord + Copy>(
    len: usize,
    key_at: impl Fn(usize) -> Option<T>,
    lower: Bound<T>,
    upper: Bound<T>,
) -> Option<Range<usize>> {
    let mut previous = None;
    for mark in 0..len {
        let key = key_at(mark)?;
        if previous.is_some_and(|last| last > key) {
            return None;
        }
        previous = Some(key);
    }

    let partition_point = |target: T, inclusive: bool| {
        let mut low = 0;
        let mut high = len;
        while low < high {
            let mid = low + (high - low) / 2;
            let key = key_at(mid).unwrap();
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
    Some(start.min(end)..end)
}

pub(super) fn u64_mark_key(index: &FixedPrimaryIndex, mark: usize, field: usize) -> Option<u64> {
    let bytes: [u8; 8] = index.field(mark, field)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

pub(super) fn i64_mark_key(index: &FixedPrimaryIndex, mark: usize, field: usize) -> Option<i64> {
    let bytes: [u8; 8] = index.field(mark, field)?.try_into().ok()?;
    Some(i64::from_le_bytes(bytes))
}
