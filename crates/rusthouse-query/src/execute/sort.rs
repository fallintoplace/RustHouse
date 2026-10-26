//! Bounded ORDER BY heap with shared sort-direction metadata.
//! Rows use Arc<[bool]> so multi-key comparisons never allocate a new
//! directions vector for every retained row.
use super::{ExecutionError, Number, compare_numbers};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::io::Write;
use std::sync::Arc;

pub(super) const MAX_SORTED_ROWS: usize = 100_000;

struct SortedRow {
    key: Number,
    extra_keys: Vec<Number>,
    values: Vec<Number>,
    sequence: usize,
    descending: bool,
    extra_descending: Arc<[bool]>,
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
        let ordering = compare_numbers(self.key, other.key);
        let ordering = if self.descending {
            ordering.reverse()
        } else {
            ordering
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
        for ((left, right), descending) in self
            .extra_keys
            .iter()
            .zip(&other.extra_keys)
            .zip(self.extra_descending.iter())
        {
            let comparison = compare_numbers(*left, *right);
            let comparison = if *descending {
                comparison.reverse()
            } else {
                comparison
            };
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
            rows: BinaryHeap::new(),
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
        let mut sorted = Self::new(limit, descending);
        sorted.requested = limit
            .map(|limit| {
                limit
                    .checked_add(offset)
                    .ok_or_else(|| ExecutionError::Invalid("LIMIT plus OFFSET overflows".into()))
            })
            .transpose()?
            .unwrap_or(usize::MAX);
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

    pub(super) fn push_with_extra(
        &mut self,
        key: Number,
        extra_keys: Vec<Number>,
        values: Vec<Number>,
    ) -> Result<(), ExecutionError> {
        if self.requested > MAX_SORTED_ROWS && self.seen >= MAX_SORTED_ROWS {
            return Err(ExecutionError::Invalid(
                "ORDER BY requires LIMIT <= 100000 for large results".into(),
            ));
        }
        let sequence = self.seen;
        self.seen = self
            .seen
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("too many rows to sort".into()))?;
        let row = SortedRow {
            key,
            extra_keys,
            values,
            sequence,
            descending: self.descending,
            extra_descending: self.extra_descending.clone(),
        };
        let retained = self.requested.min(MAX_SORTED_ROWS);
        if self.rows.len() < retained {
            self.rows.push(row);
        } else if let Some(mut worst) = self.rows.peek_mut() {
            // Reheapify only once when the incoming row beats the current worst.
            if row < *worst {
                *worst = row;
            }
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
