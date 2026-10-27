//! In-memory, bounded equi-join over validated immutable typed snapshots.
use super::{ExecutionError, Number, typed};
use crate::{JoinColumn, JoinSelect, JoinType};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

// Ordered and outer joins retain both inputs, bounded by row count and bytes.
const MAX_INPUT_ROWS: u64 = 16_384;
const MAX_RESULT_ROWS: usize = 65_536;
// Unordered INNER joins can build a larger hash side than sorted/outer joins.
// Bound retained cells as well as row count to protect against wide String rows.
const MAX_STREAM_BUILD_ROWS: u64 = 32_768;
const MAX_STREAM_BUILD_BYTES: usize = 32 * 1024 * 1024;

type Pair = (Option<usize>, Option<usize>);

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

fn base_type(value: &str) -> &str {
    value
        .strip_prefix("Nullable(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value)
}

fn resolve(
    query: &JoinSelect,
    column: &JoinColumn,
    left_names: &[String],
    right_names: &[String],
) -> Result<usize, ExecutionError> {
    let (names, start) = if column.relation == query.left_alias {
        (left_names, 0)
    } else if column.relation == query.right_alias {
        (right_names, left_names.len())
    } else {
        return Err(invalid(format!(
            "unknown JOIN qualifier: {}",
            column.relation
        )));
    };
    names
        .iter()
        .position(|name| name == &column.name)
        .map(|index| start + index)
        .ok_or_else(|| {
            invalid(format!(
                "unknown JOIN column: {}.{}",
                column.relation, column.name
            ))
        })
}

fn read_rows(
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
    // expected_rows is already bounded by row_limit, so reserve once for the build.
    let mut rows = Vec::with_capacity(expected_rows as usize);
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

fn row_key(row: &[typed::Cell], indices: &[usize]) -> Option<Vec<typed::Key>> {
    let mut result = Vec::with_capacity(indices.len());
    for &index in indices {
        let cell = &row[index];
        if matches!(cell, typed::Cell::Null)
            || matches!(cell, typed::Cell::Numeric(Number::Float(value)) if value.is_nan())
        {
            return None;
        }
        result.push(cell.key());
    }
    Some(result)
}

// Build the probe key directly from its batch columns so misses never
// materialize the full row, which may contain wide String payloads.
fn batch_key(batch: &[TypedColumn], row: usize, indices: &[usize]) -> Option<Vec<typed::Key>> {
    let mut result = Vec::with_capacity(indices.len());
    for &index in indices {
        let value = typed::cell(&batch[index], row);
        if matches!(value, typed::Cell::Null)
            || matches!(value, typed::Cell::Numeric(Number::Float(number)) if number.is_nan())
        {
            return None;
        }
        result.push(value.key());
    }
    Some(result)
}

fn cell_at<'a>(
    left_rows: &'a [Vec<typed::Cell>],
    right_rows: &'a [Vec<typed::Cell>],
    pair: &Pair,
    index: usize,
    left_width: usize,
) -> Option<&'a typed::Cell> {
    if index < left_width {
        pair.0.map(|left| &left_rows[left][index])
    } else {
        pair.1.map(|right| &right_rows[right][index - left_width])
    }
}

fn add_pair(pairs: &mut Vec<Pair>, pair: Pair) -> Result<(), ExecutionError> {
    if pairs.len() >= MAX_RESULT_ROWS {
        return Err(invalid("JOIN produces more than 65536 rows"));
    }
    pairs.push(pair);
    Ok(())
}

struct ProbeOptions {
    materialize_rows: bool,
    stop_after: Option<usize>,
    any: bool,
    null_filter: Option<(usize, bool)>,
}

/// Scan the probe side without retaining its rows or joined result pairs.
fn probe_inner<F>(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    hash: &HashMap<Vec<typed::Key>, Vec<usize>>,
    options: ProbeOptions,
    mut emit: F,
) -> Result<usize, ExecutionError>
where
    F: FnMut(&[typed::Cell], &[typed::Cell]) -> Result<(), ExecutionError>,
{
    if options.stop_after == Some(0) {
        return Ok(0);
    }
    let mut reader = TypedQueryReader::open(left_path, left_names)?;
    let mut matches = 0usize;
    'scan: while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            if let Some(key) = batch_key(&batch, index, left_keys) {
                if let Some(right_indices) = hash.get(&key) {
                    // Materialize only after a row survives the join filter.
                    let mut left_row: Option<Vec<typed::Cell>> = None;
                    for &right_index in
                        right_indices
                            .iter()
                            .take(if options.any { 1 } else { usize::MAX })
                    {
                        if let Some((column, not_null)) = options.null_filter {
                            let is_null = if column < left_names.len() {
                                matches!(typed::cell(&batch[column], index), typed::Cell::Null)
                            } else {
                                matches!(
                                    &right_rows[right_index][column - left_names.len()],
                                    typed::Cell::Null
                                )
                            };
                            if is_null == not_null {
                                continue;
                            }
                        }
                        if options.stop_after.is_some_and(|end| matches >= end) {
                            break 'scan;
                        }
                        if options.stop_after.is_none() && matches >= MAX_RESULT_ROWS {
                            return Err(invalid("JOIN produces more than 65536 rows"));
                        }
                        let projection = if options.materialize_rows {
                            left_row
                                .get_or_insert_with(|| {
                                    batch
                                        .iter()
                                        .map(|column| typed::cell(column, index))
                                        .collect()
                                })
                                .as_slice()
                        } else {
                            &[]
                        };
                        emit(projection, &right_rows[right_index])?;
                        matches += 1;
                    }
                }
            }
        }
    }
    Ok(matches)
}

/// For unordered INNER joins, build only the small right side and stream the
/// left side twice. A bounded page needs only its prefix; unbounded or
/// oversized requests still validate the 65,536-result limit before output.
struct InnerJoinColumns<'a> {
    left_names: &'a [String],
    right_names: &'a [String],
    selected: &'a [(usize, String, Option<usize>)],
    left_keys: &'a [usize],
    right_keys: &'a [usize],
    null_filter: Option<(usize, bool)>,
}

fn execute_streaming_inner<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    columns: InnerJoinColumns<'_>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let InnerJoinColumns {
        left_names,
        right_names,
        selected,
        left_keys,
        right_keys,
        null_filter,
    } = columns;
    // The probe side is scanned in batches and never retained. Keep the
    // hash-build and matched-output limits, not an artificial probe-row cap.
    let right_meta = inspect_query_source(right_path)?;
    let right_rows = read_rows(
        right_path,
        right_names,
        right_meta.rows,
        MAX_STREAM_BUILD_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    let mut hash: HashMap<Vec<typed::Key>, Vec<usize>> = HashMap::with_capacity(right_rows.len());
    for (index, row) in right_rows.iter().enumerate() {
        if let Some(key) = row_key(row, right_keys) {
            hash.entry(key).or_default().push(index);
        }
    }
    // A LIMIT that fits the output budget can page through more than
    // 65,536 matched pairs without retaining them all. Without such a LIMIT,
    // continue to reject oversized results before emitting the header.
    let stop_after = query
        .limit
        .filter(|&limit| limit <= MAX_RESULT_ROWS)
        .and_then(|limit| query.offset.checked_add(limit));
    probe_inner(
        left_path,
        left_names,
        left_keys,
        &right_rows,
        &hash,
        ProbeOptions {
            materialize_rows: false,
            stop_after,
            any: query.any,
            null_filter,
        },
        |_, _| Ok(()),
    )?;

    let header = selected
        .iter()
        .map(|(_, name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join("\t");
    writeln!(output, "{header}")?;

    let left_width = left_names.len();
    let end = query
        .offset
        .saturating_add(query.limit.unwrap_or(usize::MAX));
    let mut position = 0usize;
    probe_inner(
        left_path,
        left_names,
        left_keys,
        &right_rows,
        &hash,
        ProbeOptions {
            materialize_rows: true,
            stop_after,
            any: query.any,
            null_filter,
        },
        |left, right| {
            if position >= query.offset && position < end {
                for (index, (column, _, _)) in selected.iter().enumerate() {
                    if index != 0 {
                        output.write_all(b"\t")?;
                    }
                    let cell = if *column < left_width {
                        &left[*column]
                    } else {
                        &right[*column - left_width]
                    };
                    typed::write_value(output, cell)?;
                }
                output.write_all(b"\n")?;
            }
            position += 1;
            Ok(())
        },
    )?;
    Ok(())
}

/// Stream the right input when it is too large to retain, hashing only the
/// smaller left input. SELECT columns still follow their original left/right
/// bindings; only the unordered probe traversal changes.
fn probe_right<F>(
    right_path: &Path,
    right_names: &[String],
    right_keys: &[usize],
    left_rows: &[Vec<typed::Cell>],
    hash: &HashMap<Vec<typed::Key>, Vec<usize>>,
    options: ProbeOptions,
    mut emit: F,
) -> Result<(), ExecutionError>
where
    F: FnMut(&[typed::Cell], &[typed::Cell]) -> Result<(), ExecutionError>,
{
    if options.stop_after == Some(0) {
        return Ok(());
    }
    let mut reader = TypedQueryReader::open(right_path, right_names)?;
    let mut matches = 0usize;
    let left_width = left_rows.first().map_or(0, Vec::len);
    // Each retained left row can match only its first right row.
    let mut seen_left = options.any.then(|| vec![false; left_rows.len()]);
    'scan: while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            if let Some(key) = batch_key(&batch, index, right_keys) {
                if let Some(left_indices) = hash.get(&key) {
                    let right_row = if options.materialize_rows {
                        batch
                            .iter()
                            .map(|column| typed::cell(column, index))
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    for &left_index in left_indices {
                        if options.stop_after.is_some_and(|end| matches >= end) {
                            break 'scan;
                        }
                        if let Some(seen) = seen_left.as_mut() {
                            if seen[left_index] {
                                continue;
                            }
                            seen[left_index] = true;
                        }
                        if let Some((column, not_null)) = options.null_filter {
                            let is_null = if column < left_width {
                                matches!(&left_rows[left_index][column], typed::Cell::Null)
                            } else {
                                matches!(
                                    typed::cell(&batch[column - left_width], index),
                                    typed::Cell::Null
                                )
                            };
                            if is_null == not_null {
                                continue;
                            }
                        }
                        if options.stop_after.is_none() && matches >= MAX_RESULT_ROWS {
                            return Err(invalid("JOIN produces more than 65536 rows"));
                        }
                        emit(&left_rows[left_index], &right_row)?;
                        matches += 1;
                    }
                }
            }
        }
    }
    Ok(())
}

fn execute_streaming_right<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    columns: InnerJoinColumns<'_>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let InnerJoinColumns {
        left_names,
        right_names,
        selected,
        left_keys,
        right_keys,
        null_filter,
    } = columns;
    let left_meta = inspect_query_source(left_path)?;
    let left_rows = read_rows(
        left_path,
        left_names,
        left_meta.rows,
        MAX_STREAM_BUILD_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    let mut hash: HashMap<Vec<typed::Key>, Vec<usize>> = HashMap::with_capacity(left_rows.len());
    for (index, row) in left_rows.iter().enumerate() {
        if let Some(key) = row_key(row, left_keys) {
            hash.entry(key).or_default().push(index);
        }
    }
    let stop_after = query
        .limit
        .filter(|&limit| limit <= MAX_RESULT_ROWS)
        .and_then(|limit| query.offset.checked_add(limit));
    // Validate cardinality before sending a header or output rows.
    probe_right(
        right_path,
        right_names,
        right_keys,
        &left_rows,
        &hash,
        ProbeOptions {
            materialize_rows: false,
            stop_after,
            any: query.any,
            null_filter,
        },
        |_, _| Ok(()),
    )?;
    let header = selected
        .iter()
        .map(|(_, name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join("\t");
    writeln!(output, "{header}")?;

    let left_width = left_names.len();
    let end = query
        .offset
        .saturating_add(query.limit.unwrap_or(usize::MAX));
    let mut position = 0usize;
    probe_right(
        right_path,
        right_names,
        right_keys,
        &left_rows,
        &hash,
        ProbeOptions {
            materialize_rows: true,
            stop_after,
            any: query.any,
            null_filter,
        },
        |left, right| {
            if position >= query.offset && position < end {
                for (index, (column, _, _)) in selected.iter().enumerate() {
                    if index != 0 {
                        output.write_all(b"\t")?;
                    }
                    let value = if *column < left_width {
                        &left[*column]
                    } else {
                        &right[*column - left_width]
                    };
                    typed::write_value(output, value)?;
                }
                output.write_all(b"\n")?;
            }
            position += 1;
            Ok(())
        },
    )?;
    Ok(())
}

/// Evaluate a two-table JOIN using typed values, not serialized TSV.
pub fn execute_join<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let left_meta = inspect_query_source(left_path)?;
    let right_meta = inspect_query_source(right_path)?;
    let left_names = left_meta
        .columns
        .iter()
        .map(|col| col.name.clone())
        .collect::<Vec<_>>();
    let right_names = right_meta
        .columns
        .iter()
        .map(|col| col.name.clone())
        .collect::<Vec<_>>();
    let width = left_names.len();
    let selected: Vec<(usize, String, Option<usize>)> = if let Some(columns) = &query.columns {
        let mut selected = Vec::new();
        for col in columns {
            if col.name == "*" {
                let (start, names) = if col.relation == query.left_alias {
                    (0, &left_names)
                } else if col.relation == query.right_alias {
                    (width, &right_names)
                } else {
                    return Err(invalid(format!("unknown JOIN qualifier: {}", col.relation)));
                };
                selected.extend(names.iter().enumerate().map(|(index, name)| {
                    (start + index, format!("{}.{}", col.relation, name), None)
                }));
            } else {
                let index = resolve(query, col, &left_names, &right_names)?;
                selected.push((index, format!("{}.{}", col.relation, col.name), None));
            }
        }
        selected
    } else {
        left_names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                // A USING key appears once in SELECT *. For unmatched right
                // rows, its value comes from the right side.
                let fallback = if query.using_keys.contains(name) {
                    right_names
                        .iter()
                        .position(|right| right == name)
                        .map(|right| width + right)
                } else {
                    None
                };
                (index, format!("{}.{}", query.left_alias, name), fallback)
            })
            .chain(right_names.iter().enumerate().filter_map(|(index, name)| {
                if query.using_keys.contains(name) {
                    None
                } else {
                    Some((
                        width + index,
                        format!("{}.{}", query.right_alias, name),
                        None,
                    ))
                }
            }))
            .collect::<Vec<_>>()
    };
    let order = query
        .order_by
        .iter()
        .map(|(column, descending, nulls_first)| {
            resolve(query, column, &left_names, &right_names)
                .map(|index| (index, *descending, *nulls_first))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let null_filter = query
        .where_null
        .as_ref()
        .map(|(column, not_null)| {
            resolve(query, column, &left_names, &right_names).map(|index| (index, *not_null))
        })
        .transpose()?;
    let mut left_keys = Vec::new();
    let mut right_keys = Vec::new();
    for (first, second) in &query.keys {
        let first_index = resolve(query, first, &left_names, &right_names)?;
        let second_index = resolve(query, second, &left_names, &right_names)?;
        let (left_index, right_index) = if first_index < width && second_index >= width {
            (first_index, second_index - width)
        } else if second_index < width && first_index >= width {
            (second_index, first_index - width)
        } else {
            return Err(invalid("JOIN ON must compare one column from each table"));
        };
        let left_type = &left_meta.columns[left_index].data_type;
        let right_type = &right_meta.columns[right_index].data_type;
        if base_type(left_type) != base_type(right_type) {
            return Err(invalid(format!(
                "JOIN key types differ: {left_type} and {right_type}"
            )));
        }
        left_keys.push(left_index);
        right_keys.push(right_index);
    }

    if query.kind == JoinType::Inner && order.is_empty() {
        // A very large right table can be the streamed side if the left fits
        // the same bounded hash-build budget.
        if right_meta.rows > MAX_STREAM_BUILD_ROWS && left_meta.rows <= MAX_STREAM_BUILD_ROWS {
            return execute_streaming_right(
                query,
                left_path,
                right_path,
                InnerJoinColumns {
                    left_names: &left_names,
                    right_names: &right_names,
                    selected: &selected,
                    left_keys: &left_keys,
                    right_keys: &right_keys,
                    null_filter,
                },
                output,
            );
        }
        return execute_streaming_inner(
            query,
            left_path,
            right_path,
            InnerJoinColumns {
                left_names: &left_names,
                right_names: &right_names,
                selected: &selected,
                left_keys: &left_keys,
                right_keys: &right_keys,
                null_filter,
            },
            output,
        );
    }

    let left_rows = read_rows(
        left_path,
        &left_names,
        left_meta.rows,
        MAX_INPUT_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    let right_rows = read_rows(
        right_path,
        &right_names,
        right_meta.rows,
        MAX_INPUT_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    let mut hash: HashMap<Vec<typed::Key>, Vec<usize>> = HashMap::with_capacity(right_rows.len());
    for (index, row) in right_rows.iter().enumerate() {
        if let Some(key) = row_key(row, &right_keys) {
            hash.entry(key).or_default().push(index);
        }
    }
    let keep = |pair: &Pair| {
        null_filter.is_none_or(|(index, not_null)| {
            let is_null = cell_at(&left_rows, &right_rows, pair, index, width)
                .is_none_or(|cell| matches!(cell, typed::Cell::Null));
            is_null != not_null
        })
    };
    let mut pairs: Vec<Pair> = Vec::new();
    let mut matched_right = vec![false; right_rows.len()];
    for (left_index, row) in left_rows.iter().enumerate() {
        let mut matched = false;
        if let Some(key) = row_key(row, &left_keys) {
            if let Some(matches) = hash.get(&key) {
                for &right_index in matches.iter().take(if query.any { 1 } else { usize::MAX }) {
                    matched = true;
                    matched_right[right_index] = true;
                    let pair = (Some(left_index), Some(right_index));
                    if keep(&pair) {
                        add_pair(&mut pairs, pair)?;
                    }
                }
            }
        }
        if !matched && matches!(query.kind, JoinType::Left | JoinType::Full) {
            let pair = (Some(left_index), None);
            if keep(&pair) {
                add_pair(&mut pairs, pair)?;
            }
        }
    }
    if matches!(query.kind, JoinType::Right | JoinType::Full) {
        for (right_index, matched) in matched_right.into_iter().enumerate() {
            if !matched {
                let pair = (None, Some(right_index));
                if keep(&pair) {
                    add_pair(&mut pairs, pair)?;
                }
            }
        }
    }
    if !order.is_empty() {
        let null = typed::Cell::Null;
        pairs.sort_by(|first, second| {
            for &(index, descending, nulls_first) in &order {
                let a = cell_at(&left_rows, &right_rows, first, index, width).unwrap_or(&null);
                let b = cell_at(&left_rows, &right_rows, second, index, width).unwrap_or(&null);
                let comparison = typed::compare_order_cells(a, b, descending, nulls_first);
                if comparison != std::cmp::Ordering::Equal {
                    return comparison;
                }
            }
            std::cmp::Ordering::Equal
        });
    }
    let header = selected
        .iter()
        .map(|(_, name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join("\t");
    writeln!(output, "{header}")?;
    for pair in pairs
        .iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (position, (index, _, fallback)) in selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            let null = typed::Cell::Null;
            let value = cell_at(&left_rows, &right_rows, pair, *index, width)
                .or_else(|| {
                    fallback.and_then(|index| cell_at(&left_rows, &right_rows, pair, index, width))
                })
                .unwrap_or(&null);
            typed::write_value(output, value)?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn probe_keys_match_materialized_keys_and_skip_invalid_values() {
        let batch = vec![
            TypedColumn::NullableNumeric {
                values: NumericColumn::UInt64(vec![7, 8, 9]),
                nulls: vec![0, 0, 1],
            },
            TypedColumn::Numeric(NumericColumn::Float64(vec![2.0, f64::NAN, 4.0])),
        ];
        for row in 0..3 {
            let materialized = batch
                .iter()
                .map(|column| typed::cell(column, row))
                .collect::<Vec<_>>();
            assert!(batch_key(&batch, row, &[0, 1]) == row_key(&materialized, &[0, 1]));
        }
        assert!(batch_key(&batch, 0, &[0, 1]).is_some());
        assert!(batch_key(&batch, 1, &[0, 1]).is_none());
        assert!(batch_key(&batch, 2, &[0, 1]).is_none());
    }
}
