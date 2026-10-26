use crate::{PartError, WideMark};

/// Shared granule boundaries for every requested column in a part scan.
#[derive(Default)]
pub(crate) struct ScanLayout {
    reference_row_counts: Option<Vec<u64>>,
}

impl ScanLayout {
    pub(crate) fn validate(
        &mut self,
        name: &str,
        marks: &[WideMark],
        expected_rows: u64,
    ) -> Result<(), PartError> {
        for (index, mark) in marks.iter().enumerate() {
            if mark.rows == Some(0) && index + 1 != marks.len() {
                return Err(PartError::InvalidScan(format!(
                    "zero-row mark is not terminal for {name}"
                )));
            }
            if index > 0 {
                let previous = &marks[index - 1];
                if (mark.compressed_offset, mark.decompressed_offset)
                    < (previous.compressed_offset, previous.decompressed_offset)
                {
                    return Err(PartError::InvalidScan(format!(
                        "mark offsets are not ordered for {name}"
                    )));
                }
                if mark.rows != Some(0)
                    && (mark.compressed_offset, mark.decompressed_offset)
                        == (previous.compressed_offset, previous.decompressed_offset)
                {
                    return Err(PartError::InvalidScan(format!(
                        "duplicate live mark offsets for {name}"
                    )));
                }
            }
        }
        let row_counts = marks
            .iter()
            .map(|mark| {
                mark.rows.ok_or_else(|| {
                    PartError::InvalidScan("adaptive wide marks are required".into())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let total = row_counts
            .iter()
            .try_fold(0u64, |sum, rows| sum.checked_add(*rows));
        if total != Some(expected_rows) {
            return Err(PartError::InvalidScan(format!(
                "mark row counts do not match count.txt for {name}"
            )));
        }
        if let Some(reference) = &self.reference_row_counts {
            if reference != &row_counts {
                return Err(PartError::InvalidScan(format!(
                    "granule boundaries differ for {name}"
                )));
            }
        } else {
            self.reference_row_counts = Some(row_counts);
        }
        Ok(())
    }

    pub(crate) fn mark_count(&self) -> usize {
        self.reference_row_counts.as_ref().map_or(0, Vec::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(compressed_offset: u64, rows: u64) -> WideMark {
        WideMark {
            compressed_offset,
            decompressed_offset: 0,
            rows: Some(rows),
        }
    }

    #[test]
    fn validates_shared_granule_boundaries() {
        let mut layout = ScanLayout::default();
        assert_eq!(layout.mark_count(), 0);
        layout.validate("a", &[mark(0, 2), mark(10, 3)], 5).unwrap();
        layout.validate("b", &[mark(0, 2), mark(20, 3)], 5).unwrap();
        assert_eq!(layout.mark_count(), 2);
        assert!(layout.validate("c", &[mark(0, 3), mark(10, 2)], 5).is_err());
    }

    #[test]
    fn rejects_bad_mark_order_duplicate_offsets_and_row_counts() {
        assert!(
            ScanLayout::default()
                .validate("a", &[mark(8, 1), mark(4, 1)], 2)
                .is_err()
        );
        assert!(
            ScanLayout::default()
                .validate("a", &[mark(0, 1), mark(0, 1)], 2)
                .is_err()
        );
        assert!(
            ScanLayout::default()
                .validate("a", &[mark(0, 1)], 2)
                .is_err()
        );
        assert!(
            ScanLayout::default()
                .validate("a", &[mark(0, 0), mark(1, 1)], 1)
                .is_err()
        );
    }

    #[test]
    fn rejects_nonadaptive_marks_and_row_count_overflow() {
        let mut nonadaptive = mark(0, 1);
        nonadaptive.rows = None;
        assert!(
            ScanLayout::default()
                .validate("a", &[nonadaptive], 1)
                .is_err()
        );
        assert!(
            ScanLayout::default()
                .validate("a", &[mark(0, u64::MAX), mark(1, 1)], u64::MAX)
                .is_err()
        );
    }

    #[test]
    fn failed_column_validation_does_not_change_reference_layout() {
        let mut layout = ScanLayout::default();
        layout.validate("a", &[mark(0, 2), mark(10, 3)], 5).unwrap();
        assert!(
            layout
                .validate("bad", &[mark(0, 4), mark(10, 1)], 5)
                .is_err()
        );
        layout.validate("c", &[mark(0, 2), mark(20, 3)], 5).unwrap();
        assert_eq!(layout.mark_count(), 2);
    }
}
