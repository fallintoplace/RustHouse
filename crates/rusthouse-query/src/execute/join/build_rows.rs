use super::*;

type RowPredicate<'a> = dyn FnMut(&[typed::Cell]) -> bool + 'a;

pub(in crate::execute) fn read_rows(
    path: &Path,
    names: &[String],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    if expected_rows > row_limit {
        return Err(invalid(format!(
            "JOIN supports at most {row_limit} input rows per table"
        )));
    }
    if names.is_empty() {
        return Err(invalid("JOIN requires nonempty table schemas"));
    }
    let mut reader = TypedQueryReader::open(path, names)?;
    // Byte-budget validation, not the uncompressed row-count estimate,
    // should determine retained capacity. Avoid eagerly allocating hundreds
    // of thousands of row slots for wide tables that will fail the byte cap.
    let mut rows = Vec::with_capacity(initial_join_row_capacity(expected_rows));
    let mut retained_bytes = 0usize;
    while let Some(batch) = reader.next_batch()? {
        for row_index in 0..batch[0].len() {
            if rows.len() >= row_limit as usize {
                return Err(invalid("JOIN input row limit exceeded"));
            }
            let cells: Vec<typed::Cell> = batch
                .iter()
                .map(|column| typed::cell(column, row_index))
                .collect();
            // Include row/Cell storage and variable-width payload. The hash
            // table adds overhead, so this is a conservative input budget,
            // not a guarantee about total process RSS.
            let row_bytes = cells
                .len()
                .checked_mul(std::mem::size_of::<typed::Cell>())
                .and_then(|size| size.checked_add(std::mem::size_of::<Vec<typed::Cell>>()))
                .and_then(|size| {
                    cells.iter().try_fold(size, |total, cell| {
                        let additional = match cell {
                            typed::Cell::Text(value) => value.len(),
                            typed::Cell::Uuid(value) => value.len(),
                            _ => 0,
                        };
                        total.checked_add(additional)
                    })
                })
                .ok_or_else(|| invalid("JOIN build size overflow"))?;
            retained_bytes = retained_bytes
                .checked_add(row_bytes)
                .ok_or_else(|| invalid("JOIN build size overflow"))?;
            if retained_bytes > byte_limit {
                return Err(invalid("JOIN exceeds its retained-row memory budget"));
            }
            rows.push(cells);
        }
    }
    if rows.len() as u64 != expected_rows {
        return Err(invalid("JOIN input row count changed during scan"));
    }
    Ok(rows)
}

// Build the probe key directly from its batch columns so misses never
// materialize the full row, which may contain wide String payloads.

pub(in crate::execute) fn selected_header(selected: &[(usize, String, Option<usize>)]) -> String {
    let mut header = String::new();
    for (index, (_, name, _)) in selected.iter().enumerate() {
        if index > 0 {
            header.push('\t');
        }
        header.push_str(name);
    }
    header
}

/// Retain only join keys, projected build-side columns and the optional
/// post-join filter. All other columns are read neither from disk nor into
/// the hash build. Slot indices remain in the full table schema.
pub(in crate::execute) fn retained_build_columns(
    names: &[String],
    keys: &[usize],
    selected: &[(usize, String, Option<usize>)],
    null_filter: Option<(usize, bool)>,
    base: usize,
) -> Vec<usize> {
    let mut keep = vec![false; names.len()];
    for &key in keys {
        keep[key] = true;
    }
    for &(column, _, fallback) in selected {
        // FULL/RIGHT JOIN USING projections may read a fallback key from
        // the opposite side when the preferred side is NULL.
        for index in std::iter::once(column).chain(fallback) {
            if let Some(local) = index.checked_sub(base).filter(|&local| local < names.len()) {
                keep[local] = true;
            }
        }
    }
    if let Some((column, _)) = null_filter {
        if let Some(local) = column
            .checked_sub(base)
            .filter(|&local| local < names.len())
        {
            keep[local] = true;
        }
    }
    keep.iter()
        .enumerate()
        .filter_map(|(i, &used)| used.then_some(i))
        .collect()
}

/// Keep full-column positions for existing JOIN key/projection code, but only
/// decode retained physical streams. This is especially useful for wide
/// dimensions with many String payload columns.
pub(in crate::execute) fn read_projected_build_rows(
    path: &Path,
    names: &[String],
    indices: &[usize],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    read_projected_build_rows_using_metadata(
        path,
        names,
        indices,
        expected_rows,
        row_limit,
        byte_limit,
        None,
    )
}

/// Retain full-width rows while rejecting rows with a safe conjunctive filter.
/// Keeping the uncompressed width preserves the chained JOIN key indices.
pub(in crate::execute) fn read_projected_build_rows_matching(
    path: &Path,
    names: &[String],
    indices: &[usize],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
    predicate: &mut RowPredicate<'_>,
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    read_projected_build_rows_filtered(
        path,
        names,
        indices,
        expected_rows,
        row_limit,
        byte_limit,
        None,
        None,
        Some(predicate),
        false,
    )
}

/// Retain only projected cells while preserving their projection order.
pub(in crate::execute) fn read_filtered_compact_projected_build_rows(
    path: &Path,
    names: &[String],
    indices: &[usize],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
    filter: (usize, crate::Operator, &typed::Cell),
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    read_projected_build_rows_filtered(
        path,
        names,
        indices,
        expected_rows,
        row_limit,
        byte_limit,
        None,
        Some(filter),
        None,
        true,
    )
}

pub(in crate::execute) fn read_compact_projected_build_rows(
    path: &Path,
    names: &[String],
    indices: &[usize],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    read_projected_build_rows_filtered(
        path,
        names,
        indices,
        expected_rows,
        row_limit,
        byte_limit,
        None,
        None,
        None,
        true,
    )
}

pub(in crate::execute) fn read_compact_projected_build_rows_matching(
    path: &Path,
    names: &[String],
    indices: &[usize],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
    predicate: &mut RowPredicate<'_>,
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    read_projected_build_rows_filtered(
        path,
        names,
        indices,
        expected_rows,
        row_limit,
        byte_limit,
        None,
        None,
        Some(predicate),
        true,
    )
}

pub(in crate::execute) fn read_projected_build_rows_using_metadata(
    path: &Path,
    names: &[String],
    indices: &[usize],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
    metadata: Option<&PartMetadata>,
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    read_projected_build_rows_filtered(
        path,
        names,
        indices,
        expected_rows,
        row_limit,
        byte_limit,
        metadata,
        None,
        None,
        false,
    )
}

/// Scan every physical row, but retain only those satisfying one safe
/// conjunctive predicate. The retained row and byte limits still apply.
#[allow(clippy::too_many_arguments)]
fn read_projected_build_rows_filtered(
    path: &Path,
    names: &[String],
    indices: &[usize],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
    metadata: Option<&PartMetadata>,
    filter: Option<(usize, crate::Operator, &typed::Cell)>,
    mut row_filter: Option<&mut RowPredicate<'_>>,
    compact: bool,
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    if filter.is_none() && row_filter.is_none() && expected_rows > row_limit {
        return Err(invalid(format!(
            "JOIN supports at most {row_limit} input rows per table"
        )));
    }
    if indices.is_empty() || names.is_empty() {
        return Err(invalid("JOIN requires nonempty build columns"));
    }
    if compact
        && indices
            .iter()
            .enumerate()
            .any(|(index, column)| indices[..index].contains(column))
    {
        return Err(invalid(
            "JOIN compact projection contains duplicate columns",
        ));
    }
    let filter_column = filter
        .as_ref()
        .map(|(column, _, _)| {
            indices
                .iter()
                .position(|index| index == column)
                .ok_or_else(|| invalid("filtered JOIN column was not projected"))
        })
        .transpose()?;
    let projected = indices
        .iter()
        .map(|&i| names[i].clone())
        .collect::<Vec<_>>();
    let mut reader = if let Some(metadata) = metadata {
        TypedQueryReader::open_with_metadata(path, &projected, metadata)?
    } else {
        TypedQueryReader::open(path, &projected)?
    };
    let mut rows = Vec::with_capacity(initial_join_row_capacity(expected_rows));
    let mut retained_bytes = 0usize;
    let mut scanned = 0u64;
    while let Some(batch) = reader.next_batch()? {
        for row_index in 0..batch[0].len() {
            scanned = scanned
                .checked_add(1)
                .ok_or_else(|| invalid("JOIN input row count overflow"))?;
            if let Some((_, operator, value)) = filter {
                let index = filter_column.expect("filtered column projected");
                let candidate = typed::cell(&batch[index], row_index);
                if !typed::matches_comparison(&candidate, value, operator) {
                    continue;
                }
            }
            let mut cells = vec![typed::Cell::Null; names.len()];
            for (column, &full_index) in indices.iter().enumerate() {
                cells[full_index] = typed::cell(&batch[column], row_index);
            }
            if let Some(predicate) = row_filter.as_mut() {
                if !(**predicate)(&cells) {
                    continue;
                }
            }
            if rows.len() >= row_limit as usize {
                return Err(invalid("JOIN input row limit exceeded"));
            }
            let retained = if compact {
                indices
                    .iter()
                    .map(|&column| std::mem::replace(&mut cells[column], typed::Cell::Null))
                    .collect::<Vec<_>>()
            } else {
                cells
            };
            let bytes = retained
                .len()
                .checked_mul(std::mem::size_of::<typed::Cell>())
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Vec<typed::Cell>>()))
                .and_then(|bytes| {
                    retained.iter().try_fold(bytes, |sum, cell| {
                        let added = match cell {
                            typed::Cell::Text(value) => value.len(),
                            typed::Cell::Uuid(value) => value.len(),
                            _ => 0,
                        };
                        sum.checked_add(added)
                    })
                })
                .ok_or_else(|| invalid("JOIN build size overflow"))?;
            retained_bytes = retained_bytes
                .checked_add(bytes)
                .ok_or_else(|| invalid("JOIN build size overflow"))?;
            if retained_bytes > byte_limit {
                return Err(invalid("JOIN exceeds its retained-row memory budget"));
            }
            rows.push(retained);
        }
    }
    if scanned != expected_rows {
        return Err(invalid("JOIN input row count changed during scan"));
    }
    Ok(rows)
}

/// JOIN COUNT keeps no projected payloads. Both build orientations
/// still need equi-keys and whichever side supplies the IS NULL filter.
pub(in crate::execute) fn count_build_columns(
    keys: &[usize],
    null_filter: Option<(usize, bool)>,
    base: usize,
    width: usize,
) -> Vec<usize> {
    let mut columns = keys.to_vec();
    if let Some((column, _)) = null_filter {
        if let Some(local) = column.checked_sub(base).filter(|&i| i < width) {
            columns.push(local);
        }
    }
    columns.sort_unstable();
    columns.dedup();
    if columns.is_empty() && width > 0 {
        columns.push(0);
    }
    columns
}

#[cfg(test)]
mod row_filter_limit_tests {
    use super::*;
    use rusthouse_storage::{Column, write_wide_part};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_PART: AtomicUsize = AtomicUsize::new(0);

    fn write_rows(values: &[&str]) -> std::path::PathBuf {
        let part = std::env::temp_dir().join(format!(
            "rusthouse-join-row-filter-limit-{}-{}",
            std::process::id(),
            NEXT_PART.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = fs::remove_dir_all(&part);
        let rows = values
            .iter()
            .map(|value| vec![Some((*value).to_owned())])
            .collect::<Vec<_>>();
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt8".into(),
            }],
            &rows,
        )
        .unwrap();
        part
    }

    fn matches_one(row: &[typed::Cell]) -> bool {
        matches!(row.first(), Some(typed::Cell::Numeric(Number::Unsigned(1))))
    }

    #[test]
    fn compact_projection_retains_only_selected_cells_in_projection_order() {
        let part = std::env::temp_dir().join(format!(
            "rusthouse-compact-join-build-{}-{}",
            std::process::id(),
            NEXT_PART.fetch_add(1, Ordering::Relaxed),
        ));
        let schema = [
            Column {
                name: "id".into(),
                data_type: "UInt8".into(),
            },
            Column {
                name: "unused".into(),
                data_type: "String".into(),
            },
            Column {
                name: "label".into(),
                data_type: "String".into(),
            },
        ];
        write_wide_part(
            &part,
            &schema,
            &[vec![
                Some("1".into()),
                Some("wide payload".into()),
                Some("kept".into()),
            ]],
        )
        .unwrap();

        let names = schema
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let rows = read_compact_projected_build_rows(&part, &names, &[2, 0], 1, 1, 1024).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 2);
        assert!(matches!(&rows[0][0], typed::Cell::Text(value) if value == b"kept"));
        assert!(matches!(
            rows[0][1],
            typed::Cell::Numeric(Number::Unsigned(1))
        ));
        fs::remove_dir_all(part).unwrap();
    }

    #[test]
    fn rejected_rows_do_not_exhaust_the_retained_row_limit() {
        let part = write_rows(&["1", "0", "0"]);
        let rows = read_compact_projected_build_rows_matching(
            &part,
            &["id".into()],
            &[0],
            3,
            1,
            1024,
            &mut matches_one,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(matches_one(&rows[0]));
        fs::remove_dir_all(part).unwrap();
    }

    #[test]
    fn matching_rows_still_enforce_the_retained_row_limit() {
        let part = write_rows(&["1", "1"]);
        let result = read_compact_projected_build_rows_matching(
            &part,
            &["id".into()],
            &[0],
            2,
            1,
            1024,
            &mut matches_one,
        );
        let error = match result {
            Ok(_) => panic!("matching rows should exceed the retained row limit"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("JOIN input row limit exceeded"));
        fs::remove_dir_all(part).unwrap();
    }

    #[test]
    fn full_width_matching_rows_preserve_join_positions_and_apply_limits_after_filtering() {
        let part = write_rows(&["1", "0", "1"]);
        let names = vec!["id".into()];
        let mut predicate = matches_one;
        let rows =
            read_projected_build_rows_matching(&part, &names, &[0], 3, 2, 1024, &mut predicate)
                .unwrap();

        assert_eq!(rows.len(), 2);
        assert!(matches!(
            rows[0][0],
            typed::Cell::Numeric(Number::Unsigned(1))
        ));
        assert!(matches!(
            rows[1][0],
            typed::Cell::Numeric(Number::Unsigned(1))
        ));
        fs::remove_dir_all(part).unwrap();
    }
}
