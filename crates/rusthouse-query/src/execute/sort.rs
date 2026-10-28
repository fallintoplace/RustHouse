//! Bounded ORDER BY heap with shared sort-direction metadata.
//! Rows use Arc<[bool]> so multi-key comparisons never allocate a new
//! directions vector for every retained row.
use super::{ExecutionError, Number, compare_numbers};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::io::Write;
use std::sync::Arc;

pub(super) const MAX_SORTED_ROWS: usize = 200_000;

struct SortedRow {
    key: Number,
    extra_keys: Vec<Number>,
    values: Vec<Number>,
    sequence: usize,
    descending: bool,
    extra_descending: Arc<[bool]>,
}

fn compare_with_direction(left: Number, right: Number, descending: bool) -> Ordering {
    let ordering = compare_numbers(left, right);
    if descending {
        ordering.reverse()
    } else {
        ordering
    }
}

impl PartialEq for SortedRow {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SortedRow {}

impl PartialOrd for SortedRow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortedRow {
    fn cmp(&self, other: &Self) -> Ordering {
        let ordering = compare_with_direction(self.key, other.key, self.descending);
        if ordering != Ordering::Equal {
            return ordering;
        }
        for ((left, right), descending) in self
            .extra_keys
            .iter()
            .zip(&other.extra_keys)
            .zip(self.extra_descending.iter())
        {
            let comparison = compare_with_direction(*left, *right, *descending);
            if comparison != Ordering::Equal {
                return comparison;
            }
        }
        self.sequence.cmp(&other.sequence)
    }
}

pub(super) struct SortedRows {
    rows: BinaryHeap<SortedRow>,
    requested: usize,
    offset: usize,
    seen: usize,
    descending: bool,
    extra_descending: Arc<[bool]>,
}

impl SortedRows {
    pub(super) fn new(limit: Option<usize>, descending: bool) -> Self {
        Self {
            // Most ORDER BY queries use a small explicit LIMIT. Reserve
            // the bounded heap once, without eagerly allocating an unbounded sort.
            rows: BinaryHeap::with_capacity(limit.unwrap_or(0).min(1024)),
            requested: limit.unwrap_or(usize::MAX),
            offset: 0,
            seen: 0,
            descending,
            extra_descending: Arc::from(Vec::<bool>::new()),
        }
    }

    pub(super) fn with_offset(
        limit: Option<usize>,
        offset: usize,
        descending: bool,
    ) -> Result<Self, ExecutionError> {
        let requested = limit
            .map(|limit| {
                limit
                    .checked_add(offset)
                    .ok_or_else(|| ExecutionError::Invalid("LIMIT plus OFFSET overflows".into()))
            })
            .transpose()?
            .unwrap_or(usize::MAX);
        // Construct directly with the page size instead of allocating for
        // LIMIT and then growing immediately to accommodate OFFSET.
        let mut sorted = Self::new(limit.map(|_| requested), descending);
        sorted.requested = requested;
        sorted.offset = offset;
        Ok(sorted)
    }

    pub(super) fn requested(&self) -> usize {
        self.requested
    }

    pub(super) fn offset(&self) -> usize {
        self.offset
    }

    pub(super) fn descending(&self) -> bool {
        self.descending
    }

    pub(super) fn set_extra_directions(&mut self, directions: impl IntoIterator<Item = bool>) {
        self.extra_descending = Arc::from(directions.into_iter().collect::<Vec<_>>());
    }

    pub(super) fn push(&mut self, key: Number, values: Vec<Number>) -> Result<(), ExecutionError> {
        self.push_with_extra(key, Vec::new(), values)
    }

    /// A later candidate tied with the worst row loses the stable tie-break.
    /// Only bounded heaps can prune: unbounded scans must still enforce
    /// the maximum unsorted-input row count in push_with_extra.
    pub(super) fn would_discard(
        &self,
        key: Number,
        extra_keys: impl IntoIterator<Item = Number>,
    ) -> bool {
        if self.requested > MAX_SORTED_ROWS || self.rows.len() < self.requested {
            return false;
        }
        let Some(worst) = self.rows.peek() else {
            return false;
        };
        let primary = compare_with_direction(key, worst.key, self.descending);
        if primary != Ordering::Equal {
            return primary == Ordering::Greater;
        }
        for ((left, right), descending) in extra_keys
            .into_iter()
            .zip(&worst.extra_keys)
            .zip(self.extra_descending.iter())
        {
            let comparison = compare_with_direction(left, *right, *descending);
            if comparison != Ordering::Equal {
                return comparison == Ordering::Greater;
            }
        }
        true
    }

    pub(super) fn push_with_extra(
        &mut self,
        key: Number,
        extra_keys: Vec<Number>,
        values: Vec<Number>,
    ) -> Result<(), ExecutionError> {
        if self.requested > MAX_SORTED_ROWS && self.seen >= MAX_SORTED_ROWS {
            return Err(ExecutionError::Invalid(
                "ORDER BY requires LIMIT <= 200000 for large results".into(),
            ));
        }
        let sequence = self.seen;
        self.seen = self
            .seen
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("too many rows to sort".into()))?;
        let retained = self.requested.min(MAX_SORTED_ROWS);
        if retained == 0
            || (self.rows.len() == retained && self.would_discard(key, extra_keys.iter().copied()))
        {
            return Ok(());
        }
        let row = SortedRow {
            key,
            extra_keys,
            values,
            sequence,
            descending: self.descending,
            extra_descending: self.extra_descending.clone(),
        };
        if self.rows.len() < retained {
            self.rows.push(row);
        } else if self.requested <= MAX_SORTED_ROWS
            || self.rows.peek().is_some_and(|worst| row < *worst)
        {
            // would_discard already checked bounded candidates. Unbounded
            // scans still require the comparison before replacement.
            *self.rows.peek_mut().unwrap() = row;
        }
        Ok(())
    }

    pub(super) fn write<W: Write>(self, output: &mut W) -> Result<(), ExecutionError> {
        for row in self.rows.into_sorted_vec().into_iter().skip(self.offset) {
            for (index, value) in row.values.iter().enumerate() {
                if index != 0 {
                    write!(output, "\t")?;
                }
                write!(output, "{value}")?;
            }
            writeln!(output)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_bounded_topk_and_offset_without_eager_unbounded_allocation() {
        let limited = SortedRows::new(Some(16), false);
        assert!(limited.rows.capacity() >= 16);
        let paged = SortedRows::with_offset(Some(5), 90, true).unwrap();
        assert!(paged.rows.capacity() >= 95);
        let unbounded = SortedRows::new(None, false);
        assert_eq!(unbounded.rows.capacity(), 0);
    }

    #[test]
    fn direction_comparison_reverses_order_without_changing_ties() {
        let one = Number::Unsigned(1);
        let two = Number::Unsigned(2);
        assert_eq!(compare_with_direction(one, two, false), Ordering::Less);
        assert_eq!(compare_with_direction(one, two, true), Ordering::Greater);
        assert_eq!(compare_with_direction(one, one, true), Ordering::Equal);
    }

    #[test]
    fn heap_retains_more_than_one_hundred_thousand_rows() {
        let mut sorted = SortedRows::new(Some(100_001), false);
        for value in 0..100_001u128 {
            sorted
                .push(Number::Unsigned(value), vec![Number::Unsigned(value)])
                .unwrap();
        }
        assert_eq!(sorted.rows.len(), 100_001);
        assert_eq!(sorted.seen, 100_001);
    }

    #[test]
    fn bounded_heap_keeps_the_best_rows_with_offset_and_ties() {
        let mut sorted = SortedRows::with_offset(Some(2), 1, false).unwrap();
        for value in [10u128, 8, 1, 4, 1, 3, 2] {
            sorted
                .push(Number::Unsigned(value), vec![Number::Unsigned(value)])
                .unwrap();
        }
        let mut output = Vec::new();
        sorted.write(&mut output).unwrap();
        assert_eq!(output, b"1\n2\n");

        let mut descending = SortedRows::new(Some(2), true);
        for value in [5u128, 1, 9, 3, 9] {
            descending
                .push(Number::Unsigned(value), vec![Number::Unsigned(value)])
                .unwrap();
        }
        let mut output = Vec::new();
        descending.write(&mut output).unwrap();
        assert_eq!(output, b"9\n9\n");
    }

    #[test]
    fn bounded_topk_can_discard_losing_keys_and_later_ties() {
        let mut sorted = SortedRows::new(Some(2), false);
        sorted.set_extra_directions([true]);
        sorted
            .push_with_extra(
                Number::Unsigned(1),
                vec![Number::Unsigned(5)],
                vec![Number::Unsigned(15)],
            )
            .unwrap();
        sorted
            .push_with_extra(
                Number::Unsigned(1),
                vec![Number::Unsigned(3)],
                vec![Number::Unsigned(13)],
            )
            .unwrap();

        assert!(sorted.would_discard(Number::Unsigned(2), [Number::Unsigned(8)]));
        assert!(sorted.would_discard(Number::Unsigned(1), [Number::Unsigned(3)]));
        assert!(!sorted.would_discard(Number::Unsigned(1), [Number::Unsigned(7)]));
        sorted
            .push_with_extra(
                Number::Unsigned(1),
                vec![Number::Unsigned(7)],
                vec![Number::Unsigned(17)],
            )
            .unwrap();
        let mut output = Vec::new();
        sorted.write(&mut output).unwrap();
        assert_eq!(output, b"17\n15\n");

        let unlimited = SortedRows::new(None, false);
        assert!(!unlimited.would_discard(Number::Unsigned(1), []));
    }

    #[test]
    fn losing_primary_keys_do_not_evaluate_secondary_keys() {
        let mut sorted = SortedRows::new(Some(1), false);
        sorted.set_extra_directions([false]);
        sorted
            .push_with_extra(
                Number::Unsigned(5),
                vec![Number::Unsigned(10)],
                vec![Number::Unsigned(5)],
            )
            .unwrap();
        let mut visited = 0;
        assert!(sorted.would_discard(
            Number::Unsigned(6),
            std::iter::once_with(|| {
                visited += 1;
                Number::Unsigned(1)
            })
        ));
        assert_eq!(visited, 0);
        assert!(!sorted.would_discard(
            Number::Unsigned(5),
            std::iter::once_with(|| {
                visited += 1;
                Number::Unsigned(9)
            })
        ));
        assert_eq!(visited, 1);
    }

    #[test]
    fn rejected_topk_candidates_still_advance_stable_sequence() {
        let mut sorted = SortedRows::new(Some(1), false);
        sorted
            .push(Number::Unsigned(2), vec![Number::Unsigned(2)])
            .unwrap();
        sorted
            .push(Number::Unsigned(3), vec![Number::Unsigned(3)])
            .unwrap();
        sorted
            .push(Number::Unsigned(2), vec![Number::Unsigned(99)])
            .unwrap();
        assert_eq!(sorted.seen, 3);
        assert_eq!(sorted.rows.len(), 1);
        let mut output = Vec::new();
        sorted.write(&mut output).unwrap();
        assert_eq!(output, b"2\n");
    }

    #[test]
    fn shares_sort_directions_and_keeps_secondary_order() {
        let mut sorted = SortedRows::new(Some(3), false);
        sorted.set_extra_directions([true]);
        for number in [10u128, 30, 20] {
            sorted
                .push_with_extra(
                    Number::Unsigned(1),
                    vec![Number::Unsigned(number)],
                    vec![Number::Unsigned(number)],
                )
                .unwrap();
        }
        assert!(
            sorted
                .rows
                .iter()
                .all(|row| Arc::ptr_eq(&row.extra_descending, &sorted.extra_descending))
        );

        let mut output = Vec::new();
        sorted.write(&mut output).unwrap();
        assert_eq!(output, b"30\n20\n10\n");
    }
}
