//! In-memory, bounded equi-join over validated immutable typed snapshots.
use super::{ExecutionError, Number, typed};
use crate::{JoinColumn, JoinSelect, JoinType};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_INPUT_ROWS: u64 = 4096;
const MAX_RESULT_ROWS: usize = 65_536;
const MAX_STREAM_LEFT_ROWS: u64 = 100_000;

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
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    if expected_rows > MAX_INPUT_ROWS {
        return Err(invalid("JOIN supports at most 4096 input rows per table"));
    }
    if names.is_empty() {
        return Err(invalid("JOIN requires nonempty table schemas"));
    }
    let mut reader = TypedQueryReader::open(path, names)?;
    let mut rows = Vec::new();
    while let Some(batch) = reader.next_batch()? {
        for row_index in 0..batch[0].len() {
            if rows.len() >= MAX_INPUT_ROWS as usize {
                return Err(invalid("JOIN input row limit exceeded"));
            }
            rows.push(
                batch
                    .iter()
                    .map(|column| typed::cell(column, row_index))
                    .collect(),
            );
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

/// Scan the probe side without retaining its rows or joined result pairs.
fn probe_inner<F>(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    hash: &HashMap<Vec<typed::Key>, Vec<usize>>,
    mut emit: F,
) -> Result<usize, ExecutionError>
where
    F: FnMut(&[typed::Cell], &[typed::Cell]) -> Result<(), ExecutionError>,
{
    let mut reader = TypedQueryReader::open(left_path, left_names)?;
    let mut matches = 0usize;
    while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            let left_row = batch
                .iter()
                .map(|column| typed::cell(column, index))
                .collect::<Vec<_>>();
            if let Some(key) = row_key(&left_row, left_keys) {
                if let Some(right_indices) = hash.get(&key) {
                    for &right_index in right_indices {
                        if matches >= MAX_RESULT_ROWS {
                            return Err(invalid("JOIN produces more than 65536 rows"));
                        }
                        emit(&left_row, &right_rows[right_index])?;
                        matches += 1;
                    }
                }
            }
        }
    }
    Ok(matches)
}

/// For unordered INNER joins, build only the small right side and stream the
/// left side twice. The first pass validates the output bound before writing
/// any rows, preserving the existing no-partial-result behavior on overflow.
fn execute_streaming_inner<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    left_names: &[String],
    right_names: &[String],
    selected: &[(usize, String)],
    left_keys: &[usize],
    right_keys: &[usize],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let left_meta = inspect_query_source(left_path)?;
    if left_meta.rows > MAX_STREAM_LEFT_ROWS {
        return Err(invalid("streaming INNER JOIN supports at most 100000 left rows"));
    }
    let right_meta = inspect_query_source(right_path)?;
    let right_rows = read_rows(right_path, right_names, right_meta.rows)?;
    let mut hash: HashMap<Vec<typed::Key>, Vec<usize>> = HashMap::new();
    for (index, row) in right_rows.iter().enumerate() {
        if let Some(key) = row_key(row, right_keys) {
            hash.entry(key).or_default().push(index);
        }
    }
    probe_inner(
        left_path,
        left_names,
        left_keys,
        &right_rows,
        &hash,
        |_, _| Ok(()),
    )?;

    let header = selected
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>()
        .join("\t");
    writeln!(output, "{header}")?;

    let left_width = left_names.len();
    let end = query.offset.saturating_add(query.limit.unwrap_or(usize::MAX));
    let mut position = 0usize;
    probe_inner(
        left_path,
        left_names,
        left_keys,
        &right_rows,
        &hash,
        |left, right| {
            if position >= query.offset && position < end {
                for (index, (column, _)) in selected.iter().enumerate() {
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
    let selected =
        if let Some(columns) = &query.columns {
            columns
                .iter()
                .map(|col| {
                    resolve(query, col, &left_names, &right_names)
                        .map(|index| (index, format!("{}.{}", col.relation, col.name)))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            left_names
                .iter()
                .enumerate()
                .map(|(index, name)| (index, format!("{}.{}", query.left_alias, name)))
                .chain(right_names.iter().enumerate().map(|(index, name)| {
                    (width + index, format!("{}.{}", query.right_alias, name))
                }))
                .collect::<Vec<_>>()
        };
    let order = query
        .order_by
        .as_ref()
        .map(|(column, descending)| {
            resolve(query, column, &left_names, &right_names).map(|index| (index, *descending))
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

    if query.kind == JoinType::Inner && order.is_none() {
        return execute_streaming_inner(
            query,
            left_path,
            right_path,
            &left_names,
            &right_names,
            &selected,
            &left_keys,
            &right_keys,
            output,
        );
    }

    let left_rows = read_rows(left_path, &left_names, left_meta.rows)?;
    let right_rows = read_rows(right_path, &right_names, right_meta.rows)?;
    let mut hash: HashMap<Vec<typed::Key>, Vec<usize>> = HashMap::new();
    for (index, row) in right_rows.iter().enumerate() {
        if let Some(key) = row_key(row, &right_keys) {
            hash.entry(key).or_default().push(index);
        }
    }
    let mut pairs: Vec<Pair> = Vec::new();
    let mut matched_right = vec![false; right_rows.len()];
    for (left_index, row) in left_rows.iter().enumerate() {
        let mut matched = false;
        if let Some(key) = row_key(row, &left_keys) {
            if let Some(matches) = hash.get(&key) {
                for &right_index in matches {
                    matched = true;
                    matched_right[right_index] = true;
                    add_pair(&mut pairs, (Some(left_index), Some(right_index)))?;
                }
            }
        }
        if !matched && matches!(query.kind, JoinType::Left | JoinType::Full) {
            add_pair(&mut pairs, (Some(left_index), None))?;
        }
    }
    if matches!(query.kind, JoinType::Right | JoinType::Full) {
        for (right_index, matched) in matched_right.into_iter().enumerate() {
            if !matched {
                add_pair(&mut pairs, (None, Some(right_index)))?;
            }
        }
    }
    if let Some((index, descending)) = order {
        let null = typed::Cell::Null;
        pairs.sort_by(|first, second| {
            let a = cell_at(&left_rows, &right_rows, first, index, width).unwrap_or(&null);
            let b = cell_at(&left_rows, &right_rows, second, index, width).unwrap_or(&null);
            let comparison = typed::cmp(a, b);
            if descending {
                comparison.reverse()
            } else {
                comparison
            }
        });
    }
    let header = selected
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>()
        .join("\t");
    writeln!(output, "{header}")?;
    for pair in pairs
        .iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (position, (index, _)) in selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            let null = typed::Cell::Null;
            let value = cell_at(&left_rows, &right_rows, pair, *index, width).unwrap_or(&null);
            typed::write_value(output, value)?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}
