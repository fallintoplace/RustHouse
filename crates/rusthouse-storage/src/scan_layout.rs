use crate::{PartError, WideMark};

fn mark_offset(mark: &WideMark) -> (u64, u64) {
    (mark.compressed_offset, mark.decompressed_offset)
}

fn validate_mark_position(name: &str, marks: &[WideMark], index: usize) -> Result<(), PartError> {
    let mark = &marks[index];
    if mark.rows == Some(0) && index + 1 != marks.len() {
        return Err(PartError::InvalidScan(format!(
            "zero-row mark is not terminal for {name}"
        )));
    }
    if index > 0 {
        let current_offset = mark_offset(mark);
        let previous_offset = mark_offset(&marks[index - 1]);
        if current_offset < previous_offset {
            return Err(PartError::InvalidScan(format!(
                "mark offsets are not ordered for {name}"
            )));
        }
        if mark.rows != Some(0) && current_offset == previous_offset {
            return Err(PartError::InvalidScan(format!(
                "duplicate live mark offsets for {name}"
            )));
        }
    }

    Ok(())
}

fn row_count_mismatch(name: &str) -> PartError {
    PartError::InvalidScan(format!("mark row counts do not match count.txt for {name}"))
}

fn granule_boundary_mismatch(name: &str) -> PartError {
    PartError::InvalidScan(format!("granule boundaries differ for {name}"))
}

fn validate_mark_rows(
    name: &str,
    marks: &[WideMark],
    expected_rows: u64,
    reference: Option<&[u64]>,
) -> Result<Option<Vec<u64>>, PartError> {
    // Capture only the first column's row counts. Later columns validate
    // against the reference without allocating another boundary vector.
    let mut first_row_counts = reference.is_none().then(|| Vec::with_capacity(marks.len()));
    let mut total_rows = 0u64;
    for (index, mark) in marks.iter().enumerate() {
        validate_mark_position(name, marks, index)?;
        let rows = mark
            .rows
            .ok_or_else(|| PartError::InvalidScan("adaptive wide marks are required".into()))?;
        total_rows = total_rows.checked_add(rows).ok_or_else(|| row_count_mismatch(name))?;
        if let Some(reference) = reference {
            if reference.get(index) != Some(&rows) {
                return Err(granule_boundary_mismatch(name));
            }
        } else if let Some(first) = first_row_counts.as_mut() {
            first.push(rows);
        }
    }
    if total_rows != expected_rows {
        return Err(row_count_mismatch(name));
    }
    if reference.is_some_and(|reference| reference.len() != marks.len()) {
        return Err(granule_boundary_mismatch(name));
    }
    Ok(first_row_counts)
}

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
        let row_counts = validate_mark_rows(
            name,
            marks,
            expected_rows,
            self.reference_row_counts.as_deref(),
        )?;
        if let Some(row_counts) = row_counts {
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
    fn row_count_errors_include_the_column_name() {
        let error = ScanLayout::default()
            .validate("amount", &[mark(0, 3)], 2)
            .unwrap_err();
        assert!(matches!(
            error,
            PartError::InvalidScan(message)
                if message == "mark row counts do not match count.txt for amount"
        ));
    }

    #[test]
    fn first_failed_validation_does_not_seed_reference_boundaries() {
        let mut layout = ScanLayout::default();
        assert!(layout.validate("invalid", &[mark(0, 3)], 2).is_err());
        assert_eq!(layout.mark_count(), 0);
        layout.validate("valid", &[mark(0, 2)], 2).unwrap();
        assert_eq!(layout.mark_count(), 1);
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
    fn permits_terminal_zero_row_mark_at_same_offset() {
        let mut layout = ScanLayout::default();
        layout.validate("a", &[mark(0, 3), mark(0, 0)], 3).unwrap();
        assert_eq!(layout.mark_count(), 2);
    }

    #[test]
    fn rejects_changed_mark_counts_without_losing_reference() {
        let mut layout = ScanLayout::default();
        layout
            .validate("first", &[mark(0, 2), mark(8, 3)], 5)
            .unwrap();

        let too_few = layout.validate("short", &[mark(0, 5)], 5).unwrap_err();
        assert!(matches!(
            too_few,
            PartError::InvalidScan(message) if message.contains("granule boundaries differ")
        ));
        let too_many = layout
            .validate("long", &[mark(0, 2), mark(8, 3), mark(16, 0)], 5)
            .unwrap_err();
        assert!(matches!(
            too_many,
            PartError::InvalidScan(message) if message.contains("granule boundaries differ")
        ));

        layout
            .validate("later", &[mark(0, 2), mark(20, 3)], 5)
            .unwrap();
        assert_eq!(layout.mark_count(), 2);
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
