//! In-memory, bounded equi-join over validated immutable typed snapshots.

mod keys;

use self::keys::*;

use super::{ExecutionError, Number, typed};
use crate::{JoinColumn, JoinSelect, JoinType};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

// Ordered and outer joins retain both inputs, bounded by row count and bytes.
pub(super) const MAX_INPUT_ROWS: u64 = crate::join::MAX_BUFFERED_JOIN_ROWS;
pub(super) const MAX_RESULT_ROWS: usize = crate::join::MAX_JOIN_RESULT_ROWS;
// Unordered INNER joins can build a larger hash side than sorted/outer joins.
// Bound retained cells as well as row count to protect against wide String rows.
const MAX_STREAM_BUILD_ROWS: u64 = crate::join::MAX_STREAM_JOIN_BUILD_ROWS;
pub(super) const MAX_STREAM_BUILD_BYTES: usize = crate::join::MAX_JOIN_BUILD_BYTES;

type Pair = (Option<usize>, Option<usize>);

pub(super) fn multi_join_row_key(
    row: &[typed::Cell],
    indices: &[usize],
) -> Option<Vec<typed::Key>> {
    row_key(row, indices)
}

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

pub(super) fn read_rows(
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
    let mut rows = Vec::with_capacity(expected_rows.min(4096) as usize);
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

/// Avoid allocating a heap Vec for the common unique JOIN build key.
/// Promote to a Vec only when the build side contains duplicate keys.
enum Bucket {
    One(usize),
    Many(Vec<usize>),
}

impl Bucket {
    fn push(&mut self, value: usize) {
        match self {
            Self::One(first) => *self = Self::Many(vec![*first, value]),
            Self::Many(values) => values.push(value),
        }
    }

    fn iter(&self) -> std::slice::Iter<'_, usize> {
        match self {
            Self::One(value) => std::slice::from_ref(value).iter(),
            Self::Many(values) => values.iter(),
        }
    }
}

struct ProbeOptions {
    materialize_rows: bool,
    materialize_after: usize,
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
    hash: &HashMap<Vec<typed::Key>, Bucket>,
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
    let mut probe_key = Vec::with_capacity(left_keys.len());
    'scan: while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            if batch_key_into(&batch, index, left_keys, &mut probe_key) {
                if let Some(right_indices) = hash.get(probe_key.as_slice()) {
                    // Materialize only after a row survives the join filter.
                    let mut left_row: Option<Vec<typed::Cell>> = None;
                    for &right_index in
                        right_indices
                            .iter()
                            .take(if options.any { 1 } else { usize::MAX })
                    {
                        if let Some((column, not_null)) = options.null_filter {
                            let is_null = if column < left_names.len() {
                                typed::is_null_at(&batch[column], index)
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
                            return Err(invalid("JOIN produces more than 131072 rows"));
                        }
                        let projection =
                            if options.materialize_rows && matches >= options.materialize_after {
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

/// Stream a large LEFT JOIN probe table without retaining its rows or
/// unmatched pairs. Hash match detection precedes the optional IS NULL filter.
fn probe_left_outer<F>(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    right_width: usize,
    hash: &HashMap<Vec<typed::Key>, Bucket>,
    options: ProbeOptions,
    mut emit: F,
) -> Result<usize, ExecutionError>
where
    F: FnMut(&[typed::Cell], &[typed::Cell]) -> Result<(), ExecutionError>,
{
    if options.stop_after == Some(0) { return Ok(0); }
    let mut reader = TypedQueryReader::open(left_path, left_names)?;
    let null_right = vec![typed::Cell::Null; right_width];
    let mut matches = 0usize;
    let mut probe_key = Vec::with_capacity(left_keys.len());
    'scan: while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            let candidates = if batch_key_into(&batch, row, left_keys, &mut probe_key) {
                hash.get(probe_key.as_slice())
            } else { None };
            let matched = candidates.is_some();
            let mut left_row: Option<Vec<typed::Cell>> = None;
            // When a key matches, LEFT JOIN does not emit an extra NULL row
            // even if a subsequent WHERE filter removes every matched pair.
            let candidates = candidates.into_iter().flat_map(|bucket| {
                bucket.iter().take(if options.any { 1 } else { usize::MAX })
                    .map(Some)
            });
            for right_index in candidates.chain((!matched).then_some(None)) {
                let right = right_index.map_or(null_right.as_slice(),
                    |&index| right_rows[index].as_slice());
                if let Some((column, not_null)) = options.null_filter {
                    let is_null = if column < left_names.len() {
                        typed::is_null_at(&batch[column], row)
                    } else {
                        matches!(&right[column - left_names.len()], typed::Cell::Null)
                    };
                    if is_null == not_null { continue; }
                }
                if options.stop_after.is_some_and(|end| matches >= end) {
                    break 'scan;
                }
                if options.stop_after.is_none() && matches >= MAX_RESULT_ROWS {
                    return Err(invalid("JOIN produces more than 131072 rows"));
                }
                let projection = if options.materialize_rows
                    && matches >= options.materialize_after {
                    left_row.get_or_insert_with(|| {
                        batch.iter().map(|column| typed::cell(column, row)).collect()
                    }).as_slice()
                } else { &[] };
                emit(projection, right)?;
                matches += 1;
            }
        }
    }
    Ok(matches)
}

fn execute_streaming_left<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    columns: InnerJoinColumns<'_>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let InnerJoinColumns {
        left_names, right_names, selected, left_keys, right_keys, null_filter,
    } = columns;
    let right_meta = inspect_query_source(right_path)?;
    let right_rows = read_rows(
        right_path, right_names, right_meta.rows,
        MAX_STREAM_BUILD_ROWS, MAX_STREAM_BUILD_BYTES,
    )?;
    let mut hash = HashMap::<Vec<typed::Key>, Bucket>::with_capacity(right_rows.len());
    for (index, row) in right_rows.iter().enumerate() {
        if let Some(key) = row_key(row, right_keys) {
            if query.any {
                hash.entry(key).or_insert(Bucket::One(index));
            } else {
                hash.entry(key).and_modify(|bucket| bucket.push(index))
                    .or_insert(Bucket::One(index));
            }
        }
    }
    let stop_after = query.limit.filter(|&limit| limit <= MAX_RESULT_ROWS)
        .and_then(|limit| query.offset.checked_add(limit));
    let preflight = probe_left_outer(
        left_path, left_names, left_keys, &right_rows, right_names.len(), &hash,
        ProbeOptions {
            materialize_rows: false, materialize_after: 0,
            stop_after, any: query.any, null_filter,
        },
        |_, _| Ok(()),
    )?;
    let header = selected.iter().map(|(_, name, _)| name.as_str())
        .collect::<Vec<_>>().join("\t");
    writeln!(output, "{header}")?;
    if preflight <= query.offset { return Ok(()); }
    let end = query.offset.saturating_add(query.limit.unwrap_or(usize::MAX));
    let mut seen = 0usize;
    probe_left_outer(
        left_path, left_names, left_keys, &right_rows, right_names.len(), &hash,
        ProbeOptions {
            materialize_rows: selected.iter().any(|(column, _, _)| *column < left_names.len()),
            materialize_after: query.offset, stop_after, any: query.any, null_filter,
        },
        |left, right| {
            if seen >= query.offset && seen < end {
                for (pos, (column, _, _)) in selected.iter().enumerate() {
                    if pos > 0 { output.write_all(b"\t")?; }
                    let value = if *column < left_names.len() {
                        &left[*column]
                    } else {
                        &right[*column - left_names.len()]
                    };
                    typed::write_value(output, value)?;
                }
                output.write_all(b"\n")?;
            }
            seen += 1;
            Ok(())
        },
    )?;
    Ok(())
}

/// For unordered INNER joins, build only the small right side and stream the
/// left side twice. A bounded page needs only its prefix; unbounded or
/// oversized requests still validate the 131,072-result limit before output.
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
    let mut hash: HashMap<Vec<typed::Key>, Bucket> = HashMap::with_capacity(right_rows.len());
    for (index, row) in right_rows.iter().enumerate() {
        if let Some(key) = row_key(row, right_keys) {
            // ANY keeps the first right match only, so duplicate indexes
            // never need to be retained in the hash table.
            if query.any {
                hash.entry(key).or_insert(Bucket::One(index));
            } else {
                hash.entry(key)
                    .and_modify(|bucket| bucket.push(index))
                    .or_insert(Bucket::One(index));
            }
        }
    }
    if hash.is_empty() {
        let header = selected
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join("\t");
        writeln!(output, "{header}")?;
        return Ok(());
    }
    // A LIMIT that fits the output budget can page through more than
    // 131,072 matched pairs without retaining them all. Without such a LIMIT,
    // continue to reject oversized results before emitting the header.
    let stop_after = query
        .limit
        .filter(|&limit| limit <= MAX_RESULT_ROWS)
        .and_then(|limit| query.offset.checked_add(limit));
    let preflight_matches = probe_inner(
        left_path,
        left_names,
        left_keys,
        &right_rows,
        &hash,
        ProbeOptions {
            materialize_rows: false,
            materialize_after: 0,
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
    if preflight_matches <= query.offset {
        return Ok(());
    }

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
            materialize_rows: selected.iter().any(|(column, _, _)| *column < left_width),
            materialize_after: query.offset,
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
    hash: &HashMap<Vec<typed::Key>, Bucket>,
    options: ProbeOptions,
    mut emit: F,
) -> Result<usize, ExecutionError>
where
    F: FnMut(&[typed::Cell], &[typed::Cell]) -> Result<(), ExecutionError>,
{
    if options.stop_after == Some(0) {
        return Ok(0);
    }
    let mut reader = TypedQueryReader::open(right_path, right_names)?;
    let mut matches = 0usize;
    let left_width = left_rows.first().map_or(0, Vec::len);
    // Each retained left row can match only its first right row.
    let mut seen_left = options.any.then(|| vec![false; left_rows.len()]);
    let mut remaining_left = seen_left.as_ref().map(|_| {
        hash.values()
            .map(|bucket| bucket.iter().count())
            .sum::<usize>()
    });
    let mut probe_key = Vec::with_capacity(right_keys.len());
    'scan: while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            if batch_key_into(&batch, index, right_keys, &mut probe_key) {
                if let Some(left_indices) = hash.get(probe_key.as_slice()) {
                    // Most matches may be filtered or already claimed by ANY.
                    // Decode the full probe row only when a match survives.
                    let mut right_row: Option<Vec<typed::Cell>> = None;
                    for &left_index in left_indices.iter() {
                        if options.stop_after.is_some_and(|end| matches >= end) {
                            break 'scan;
                        }
                        if let Some(seen) = seen_left.as_mut() {
                            if seen[left_index] {
                                continue;
                            }
                            seen[left_index] = true;
                            if let Some(remaining) = remaining_left.as_mut() {
                                *remaining -= 1;
                            }
                        }
                        if let Some((column, not_null)) = options.null_filter {
                            let is_null = if column < left_width {
                                matches!(&left_rows[left_index][column], typed::Cell::Null)
                            } else {
                                typed::is_null_at(&batch[column - left_width], index)
                            };
                            if is_null == not_null {
                                continue;
                            }
                        }
                        if options.stop_after.is_none() && matches >= MAX_RESULT_ROWS {
                            return Err(invalid("JOIN produces more than 131072 rows"));
                        }
                        let projection =
                            if options.materialize_rows && matches >= options.materialize_after {
                                right_row
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
                        emit(&left_rows[left_index], projection)?;
                        matches += 1;
                    }
                    if remaining_left == Some(0) {
                        break 'scan;
                    }
                }
            }
        }
    }
    Ok(matches)
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
    let mut hash: HashMap<Vec<typed::Key>, Bucket> = HashMap::with_capacity(left_rows.len());
    for (index, row) in left_rows.iter().enumerate() {
        if let Some(key) = row_key(row, left_keys) {
            hash.entry(key)
                .and_modify(|bucket| bucket.push(index))
                .or_insert(Bucket::One(index));
        }
    }
    if hash.is_empty() {
        let header = selected
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join("\t");
        writeln!(output, "{header}")?;
        return Ok(());
    }
    let stop_after = query
        .limit
        .filter(|&limit| limit <= MAX_RESULT_ROWS)
        .and_then(|limit| query.offset.checked_add(limit));
    // Validate cardinality before sending a header or output rows.
    let preflight_matches = probe_right(
        right_path,
        right_names,
        right_keys,
        &left_rows,
        &hash,
        ProbeOptions {
            materialize_rows: false,
            materialize_after: 0,
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
    if preflight_matches <= query.offset {
        return Ok(());
    }

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
            materialize_rows: selected.iter().any(|(column, _, _)| *column >= left_width),
            materialize_after: query.offset,
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
        let mut selected = Vec::new();
        // SQL USING projects shared keys first, in USING list order.
        // The fallback preserves unmatched right keys for RIGHT/FULL joins.
        for name in &query.using_keys {
            let left_index = left_names
                .iter()
                .position(|column| column == name)
                .ok_or_else(|| {
                    invalid(format!("unknown JOIN column: {}.{name}", query.left_alias))
                })?;
            let right_index = right_names
                .iter()
                .position(|column| column == name)
                .ok_or_else(|| {
                    invalid(format!("unknown JOIN column: {}.{name}", query.right_alias))
                })?;
            selected.push((
                left_index,
                format!("{}.{}", query.left_alias, name),
                Some(width + right_index),
            ));
        }
        for (index, name) in left_names.iter().enumerate() {
            if !query.using_keys.contains(name) {
                selected.push((index, format!("{}.{}", query.left_alias, name), None));
            }
        }
        for (index, name) in right_names.iter().enumerate() {
            if !query.using_keys.contains(name) {
                selected.push((
                    width + index,
                    format!("{}.{}", query.right_alias, name),
                    None,
                ));
            }
        }
        selected
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

    // LIMIT 0 needs the validated output schema, but not either input scan.
    if query.limit == Some(0) {
        let header = selected
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join("\t");
        writeln!(output, "{header}")?;
        return Ok(());
    }

    // A validated INNER join with an empty input cannot produce matches.
    if query.kind == JoinType::Inner && (left_meta.rows == 0 || right_meta.rows == 0) {
        let header = selected
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join("\t");
        writeln!(output, "{header}")?;
        return Ok(());
    }

    if query.kind == JoinType::Left && order.is_empty()
        && right_meta.rows <= MAX_STREAM_BUILD_ROWS
        && left_meta.rows > MAX_INPUT_ROWS
    {
        return execute_streaming_left(
            query, left_path, right_path,
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
    let mut hash: HashMap<Vec<typed::Key>, Bucket> = HashMap::with_capacity(right_rows.len());
    for (index, row) in right_rows.iter().enumerate() {
        if let Some(key) = row_key(row, &right_keys) {
            hash.entry(key)
                .and_modify(|bucket| bucket.push(index))
                .or_insert(Bucket::One(index));
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
    // Reuse JOIN probe storage instead of allocating a fresh key Vec per left row.
    let mut lookup_key = Vec::with_capacity(left_keys.len());
    let mut matched_right = matches!(query.kind, JoinType::Right | JoinType::Full)
        .then(|| vec![false; right_rows.len()]);
    for (left_index, row) in left_rows.iter().enumerate() {
        let mut matched = false;
        if row_key_into(row, &left_keys, &mut lookup_key) {
            if let Some(matches) = hash.get(lookup_key.as_slice()) {
                for &right_index in
                    matches
                        .iter()
                        .take(if query.any && query.kind != JoinType::Right {
                            1
                        } else {
                            usize::MAX
                        })
                {
                    if query.any
                        && query.kind == JoinType::Right
                        && matched_right
                            .as_ref()
                            .is_some_and(|flags| flags[right_index])
                    {
                        continue;
                    }
                    matched = true;
                    if let Some(flags) = matched_right.as_mut() {
                        flags[right_index] = true;
                    }
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
    if let Some(matched_right) = matched_right {
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
    fn streaming_left_outer_emits_unmatched_rows_without_duplicates() {
        use rusthouse_storage::{Column, write_wide_part};
        use std::fs;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rusthouse-left-stream-test-{}-{}",
            std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let left = root.join("left");
        let schema = [Column { name: "id".into(), data_type: "UInt64".into() }];
        write_wide_part(&left, &schema, &[
            vec![Some("3".into())], vec![Some("1".into())],
            vec![Some("2".into())],
        ]).unwrap();
        let right_rows = vec![
            vec![typed::Cell::Numeric(Number::Unsigned(1))],
            vec![typed::Cell::Numeric(Number::Unsigned(1))],
        ];
        let mut hash = HashMap::new();
        hash.insert(vec![typed::Cell::Numeric(Number::Unsigned(1)).key()],
            Bucket::Many(vec![0, 1]));
        let mut seen = Vec::new();
        let count = probe_left_outer(
            &left, &["id".into()], &[0], &right_rows, 1, &hash,
            ProbeOptions {
                materialize_rows: true, materialize_after: 0,
                stop_after: None, any: false, null_filter: None,
            },
            |left, right| {
                let id = match &left[0] {
                    typed::Cell::Numeric(Number::Unsigned(id)) => *id,
                    _ => panic!("expected numeric left key"),
                };
                seen.push((id, matches!(&right[0], typed::Cell::Null)));
                Ok(())
            },
        ).unwrap();
        assert_eq!(count, 4);
        assert_eq!(seen.iter().filter(|(_, null)| *null).count(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn probe_keys_match_materialized_keys_and_skip_invalid_values() {
        let batch = vec![
            TypedColumn::NullableNumeric {
                values: NumericColumn::UInt64(vec![7, 8, 9]),
                nulls: vec![0, 0, 1],
            },
            TypedColumn::Numeric(NumericColumn::Float64(vec![2.0, f64::NAN, 4.0])),
        ];
        let mut reused = Vec::new();
        for row in 0..3 {
            let materialized = batch
                .iter()
                .map(|column| typed::cell(column, row))
                .collect::<Vec<_>>();
            let valid = batch_key_into(&batch, row, &[0, 1], &mut reused);
            assert_eq!(valid, row_key(&materialized, &[0, 1]).is_some());
            if let Some(expected) = row_key(&materialized, &[0, 1]) {
                assert!(reused == expected);
            } else {
                assert!(reused.is_empty());
            }
        }
        assert!(batch_key_into(&batch, 0, &[0, 1], &mut reused));
        assert_eq!(reused.len(), 2);
        assert!(!batch_key_into(&batch, 1, &[0, 1], &mut reused));
        assert!(reused.is_empty());
        assert!(!batch_key_into(&batch, 2, &[0, 1], &mut reused));
        assert!(reused.is_empty());
    }
}

#[cfg(test)]
mod inline_bucket_tests {
    use super::Bucket;

    #[test]
    fn unique_keys_remain_inline_and_duplicates_keep_insertion_order() {
        let mut bucket = Bucket::One(7);
        assert_eq!(bucket.iter().copied().collect::<Vec<_>>(), vec![7]);
        bucket.push(11);
        bucket.push(13);
        assert_eq!(bucket.iter().copied().collect::<Vec<_>>(), vec![7, 11, 13]);
        assert_eq!(bucket.iter().take(1).copied().collect::<Vec<_>>(), vec![7]);
    }
}
