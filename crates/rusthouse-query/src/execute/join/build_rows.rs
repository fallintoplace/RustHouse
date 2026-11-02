use super::*;

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

/// JOIN plans that already inspected their immutable snapshot do not need to
/// reopen and validate its part metadata a second time.
pub(in crate::execute) fn read_projected_build_rows_using_metadata(
    path: &Path,
    names: &[String],
    indices: &[usize],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
    metadata: Option<&PartMetadata>,
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    if expected_rows > row_limit {
        return Err(invalid(format!(
            "JOIN supports at most {row_limit} input rows per table"
        )));
    }
    if indices.is_empty() || names.is_empty() {
        return Err(invalid("JOIN requires nonempty build columns"));
    }
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
    while let Some(batch) = reader.next_batch()? {
        for row_index in 0..batch[0].len() {
            if rows.len() >= row_limit as usize {
                return Err(invalid("JOIN input row limit exceeded"));
            }
            let mut cells = vec![typed::Cell::Null; names.len()];
            for (column, &full_index) in indices.iter().enumerate() {
                cells[full_index] = typed::cell(&batch[column], row_index);
            }
            let bytes = cells
                .len()
                .checked_mul(std::mem::size_of::<typed::Cell>())
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Vec<typed::Cell>>()))
                .and_then(|bytes| {
                    cells.iter().try_fold(bytes, |sum, cell| {
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
            rows.push(cells);
        }
    }
    if rows.len() as u64 != expected_rows {
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
