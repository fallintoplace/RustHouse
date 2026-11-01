use super::*;

fn mark_keys_sorted<T: Ord + Copy>(len: usize, key_at: &impl Fn(usize) -> Option<T>) -> bool {
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
    mark_keys_sorted(len, &key_at).then(|| candidate_mark_range_sorted(len, &key_at, lower, upper))
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
    if !mark_keys_sorted(len, &key_at) {
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
    // IN and OR ranges are often already ordered. Skip the n log n sort
    // when their candidate mark ranges arrive in ascending order.
    if ranges.windows(2).any(|pair| pair[0].start > pair[1].start) {
        ranges.sort_unstable_by_key(|range| range.start);
    }
    // Merge the sorted mark ranges in place. The original code allocated
    // another vector and copied every candidate range into it. IN/OR scans
    // can contain many intervals, but each merged range occupies at most one
    // of the already allocated input slots.
    let mut output_len = 0;
    for input in 0..ranges.len() {
        let range = ranges[input].clone();
        if output_len != 0 && range.start <= ranges[output_len - 1].end {
            ranges[output_len - 1].end = ranges[output_len - 1].end.max(range.end);
        } else {
            ranges[output_len] = range;
            output_len += 1;
        }
    }
    ranges.truncate(output_len);
    Some(ranges)
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

/// Decode a narrow physical key without sign loss or host-endian assumptions.
pub(super) fn narrow_unsigned_mark_key(
    index: &FixedPrimaryIndex, mark: usize, field: usize,
) -> Option<u64> {
    match index.field(mark, field)? {
        [a] => Some(u64::from(*a)),
        [a, b] => Some(u64::from(u16::from_le_bytes([*a, *b]))),
        [a, b, c, d] => Some(u64::from(u32::from_le_bytes([*a, *b, *c, *d]))),
        _ => None,
    }
}

pub(super) fn narrow_signed_mark_key(
    index: &FixedPrimaryIndex, mark: usize, field: usize,
) -> Option<i64> {
    match index.field(mark, field)? {
        [a] => Some(i64::from(i8::from_le_bytes([*a]))),
        [a, b] => Some(i64::from(i16::from_le_bytes([*a, *b]))),
        [a, b, c, d] => Some(i64::from(i32::from_le_bytes([*a, *b, *c, *d]))),
        _ => None,
    }
}

#[cfg(test)]
mod ordered_intervals_tests {
    use super::*;

    #[test]
    fn sorted_and_unsorted_interval_inputs_have_identical_mark_ranges() {
        let sorted = [
            (Bound::Included(10_u64), Bound::Included(15_u64)),
            (Bound::Included(14_u64), Bound::Included(22_u64)),
            (Bound::Included(20_u64), Bound::Included(25_u64)),
            (Bound::Included(40_u64), Bound::Included(42_u64)),
        ];
        let unsorted = [sorted[3], sorted[1], sorted[0], sorted[2]];
        let plan = |intervals: &[(Bound<u64>, Bound<u64>)]| {
            candidate_mark_ranges(64, |mark| Some(mark as u64), intervals).unwrap()
        };
        assert_eq!(plan(&sorted), vec![9..26, 39..43]);
        assert_eq!(plan(&unsorted), plan(&sorted));
    }
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

#[cfg(test)]
mod in_place_interval_union_tests {
    use super::*;

    #[test]
    fn large_interval_sets_match_sorted_and_reversed_reference() {
        // A mark belongs to the candidate set when any interval selects it,
        // including its predecessor granule. Cross-check every mark rather
        // than testing only the number of returned ranges.
        let intervals = (0..4096usize)
            .map(|index| {
                let start = ((index * 47) % 8192) as u64;
                (
                    Bound::Included(start),
                    Bound::Included(start + (index % 19) as u64),
                )
            })
            .collect::<Vec<_>>();
        let expected = {
            let mut selected = [false; 8192];
            for &(lower, upper) in &intervals {
                let interval =
                    candidate_mark_range(selected.len(), |mark| Some(mark as u64), lower, upper)
                        .unwrap();
                for mark in interval {
                    selected[mark] = true;
                }
            }
            selected
        };
        for unsorted in [false, true] {
            let mut input = intervals.clone();
            if unsorted {
                input.reverse();
            }
            let actual = candidate_mark_ranges(8192, |mark| Some(mark as u64), &input).unwrap();
            assert!(actual.windows(2).all(|r| r[0].end < r[1].start));
            let mut selected = [false; 8192];
            for range in actual {
                for mark in range {
                    selected[mark] = true;
                }
            }
            assert_eq!(selected, expected);
        }
    }

    #[test]
    fn nested_and_disjoint_intervals_keep_only_one_copy_of_each_mark() {
        let input = [
            (Bound::Included(20u64), Bound::Included(25u64)),
            (Bound::Included(21), Bound::Included(22)),
            (Bound::Included(40), Bound::Included(41)),
            (Bound::Included(24), Bound::Included(27)),
        ];
        assert_eq!(
            candidate_mark_ranges(100, |mark| Some(mark as u64), &input).unwrap(),
            vec![19..28, 39..42]
        );
        // The last mark is conservatively included as predecessor when
        // every indexed key lies below the lower bound.
        assert_eq!(
            candidate_mark_ranges(
                100,
                |mark| Some(mark as u64),
                &[(Bound::Included(200u64), Bound::Included(300))]
            )
            .unwrap(),
            vec![99..100]
        );
    }
}
