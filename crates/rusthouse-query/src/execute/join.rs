//! In-memory, bounded equi-join over validated immutable typed snapshots.

mod keys;
mod grouped;

use self::keys::*;

use super::{ExecutionError, Number, typed};
use crate::{JoinAggregate, JoinColumn, JoinSelect, JoinType};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::io::Write;
use std::sync::Arc;
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

/// Retain only join keys, projected build-side columns and the optional
/// post-join filter. All other columns are read neither from disk nor into
/// the hash build. Slot indices remain in the full table schema.
fn retained_build_columns(
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
    for &(column, _, _) in selected {
        if let Some(local) = column.checked_sub(base).filter(|&local| local < names.len()) {
            keep[local] = true;
        }
    }
    if let Some((column, _)) = null_filter {
        if let Some(local) = column.checked_sub(base).filter(|&local| local < names.len()) {
            keep[local] = true;
        }
    }
    keep.iter().enumerate().filter_map(|(i, &used)| used.then_some(i)).collect()
}

/// Keep full-column positions for existing JOIN key/projection code, but only
/// decode retained physical streams. This is especially useful for wide
/// dimensions with many String payload columns.
fn read_projected_build_rows(
    path: &Path,
    names: &[String],
    indices: &[usize],
    expected_rows: u64,
    row_limit: u64,
    byte_limit: usize,
) -> Result<Vec<Vec<typed::Cell>>, ExecutionError> {
    if expected_rows > row_limit {
        return Err(invalid(format!("JOIN supports at most {row_limit} input rows per table")));
    }
    if indices.is_empty() || names.is_empty() {
        return Err(invalid("JOIN requires nonempty build columns"));
    }
    let projected = indices.iter().map(|&i| names[i].clone()).collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(path, &projected)?;
    let mut rows = Vec::with_capacity(expected_rows as usize);
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
                .and_then(|bytes| cells.iter().try_fold(bytes, |sum, cell| {
                    let added = match cell {
                        typed::Cell::Text(value) => value.len(),
                        typed::Cell::Uuid(value) => value.len(),
                        _ => 0,
                    };
                    sum.checked_add(added)
                }))
                .ok_or_else(|| invalid("JOIN build size overflow"))?;
            retained_bytes = retained_bytes.checked_add(bytes)
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
fn count_build_columns(
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
    // A COUNT or preflight probe needs join keys and filters only. Keep
    // original slot positions when actual output rows must be materialized.
    let mut retained = if options.materialize_rows {
        (0..left_names.len()).collect::<Vec<_>>()
    } else {
        left_keys.to_vec()
    };
    if let Some((column, _)) = options.null_filter {
        if column < left_names.len() {
            retained.push(column);
        }
    }
    retained.sort_unstable();
    retained.dedup();
    if retained.is_empty() {
        retained.push(0); // CROSS JOIN still needs a row-count source.
    }
    let selected = retained.iter().map(|&index| left_names[index].clone()).collect::<Vec<_>>();
    let key_indices = left_keys.iter().map(|index| {
        retained.binary_search(index).expect("join key retained")
    }).collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(left_path, &selected)?;
    let mut matches = 0usize;
    let mut probe_key = Vec::with_capacity(left_keys.len());
    'scan: while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            if batch_key_into(&batch, index, &key_indices, &mut probe_key) {
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
                                typed::is_null_at(
                                    &batch[retained.binary_search(&column).expect("filter retained")],
                                    index,
                                )
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

/// Stream the right side of an unordered RIGHT OUTER JOIN, retaining only
/// a bounded left hash build and the row currently being probed.
fn probe_right_outer<F>(
    right_path: &Path,
    right_names: &[String],
    right_keys: &[usize],
    left_rows: &[Vec<typed::Cell>],
    left_width: usize,
    hash: &HashMap<Vec<typed::Key>, Bucket>,
    options: ProbeOptions,
    mut emit: F,
) -> Result<usize, ExecutionError>
where
    F: FnMut(&[typed::Cell], &[typed::Cell]) -> Result<(), ExecutionError>,
{
    if options.stop_after == Some(0) { return Ok(0); }
    let mut retained = if options.materialize_rows {
        (0..right_names.len()).collect::<Vec<_>>()
    } else {
        right_keys.to_vec()
    };
    if let Some((column, _)) = options.null_filter {
        if let Some(local) = column.checked_sub(left_width)
            .filter(|&local| local < right_names.len()) {
            retained.push(local);
        }
    }
    retained.sort_unstable();
    retained.dedup();
    if retained.is_empty() {
        retained.push(0);
    }
    let selected = retained.iter().map(|&index| right_names[index].clone()).collect::<Vec<_>>();
    let key_indices = right_keys.iter().map(|index| {
        retained.binary_search(index).expect("join key retained")
    }).collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(right_path, &selected)?;
    let null_left = vec![typed::Cell::Null; left_width];
    let mut total = 0usize;
    let mut probe_key = Vec::with_capacity(right_keys.len());
    'scan: while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            let candidates = if batch_key_into(&batch, row, &key_indices, &mut probe_key) {
                hash.get(probe_key.as_slice())
            } else { None };
            let matched = candidates.is_some();
            let mut right_row: Option<Vec<typed::Cell>> = None;
            let matches = candidates.into_iter().flat_map(|bucket| {
                bucket.iter().take(if options.any { 1 } else { usize::MAX })
                    .map(Some)
            });
            for left_index in matches.chain((!matched).then_some(None)) {
                let left = left_index.map_or(null_left.as_slice(),
                    |&index| left_rows[index].as_slice());
                if let Some((column, not_null)) = options.null_filter {
                    let is_null = if column < left_width {
                        matches!(&left[column], typed::Cell::Null)
                    } else {
                        typed::is_null_at(
                            &batch[retained.binary_search(&(column - left_width)).expect("filter retained")],
                            row,
                        )
                    };
                    if is_null == not_null { continue; }
                }
                if options.stop_after.is_some_and(|end| total >= end) {
                    break 'scan;
                }
                if options.stop_after.is_none() && total >= MAX_RESULT_ROWS {
                    return Err(invalid("JOIN produces more than 131072 rows"));
                }
                let right = if options.materialize_rows
                    && total >= options.materialize_after {
                    right_row.get_or_insert_with(|| {
                        batch.iter().map(|column| typed::cell(column, row)).collect()
                    }).as_slice()
                } else { &[] };
                emit(left, right)?;
                total += 1;
            }
        }
    }
    Ok(total)
}

fn execute_streaming_right_outer<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    columns: InnerJoinColumns<'_>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let InnerJoinColumns {
        left_names, right_names, selected, left_keys, right_keys, null_filter,
    } = columns;
    let left_meta = inspect_query_source(left_path)?;
    let keep = retained_build_columns(left_names, left_keys, selected, null_filter, 0);
    let left_rows = read_projected_build_rows(
        left_path, left_names, &keep, left_meta.rows,
        MAX_STREAM_BUILD_ROWS, MAX_STREAM_BUILD_BYTES,
    )?;
    let mut hash = HashMap::<Vec<typed::Key>, Bucket>::with_capacity(left_rows.len());
    for (index, row) in left_rows.iter().enumerate() {
        if let Some(key) = row_key(row, left_keys) {
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
    let preflight = probe_right_outer(
        right_path, right_names, right_keys, &left_rows, left_names.len(), &hash,
        ProbeOptions {
            materialize_rows: false, materialize_after: 0, stop_after,
            any: query.any, null_filter,
        }, |_, _| Ok(()),
    )?;
    let header = selected.iter().map(|(_, name, _)| name.as_str())
        .collect::<Vec<_>>().join("\t");
    writeln!(output, "{header}")?;
    if preflight <= query.offset { return Ok(()); }
    let end = query.offset.saturating_add(query.limit.unwrap_or(usize::MAX));
    let mut seen = 0usize;
    probe_right_outer(
        right_path, right_names, right_keys, &left_rows, left_names.len(), &hash,
        ProbeOptions {
            materialize_rows: selected.iter().any(|(column, _, fallback)| {
                *column >= left_names.len() || fallback.is_some()
            }),
            materialize_after: query.offset, stop_after, any: query.any, null_filter,
        },
        |left, right| {
            if seen >= query.offset && seen < end {
                for (pos, (column, _, fallback)) in selected.iter().enumerate() {
                    if pos > 0 { output.write_all(b"\t")?; }
                    let value = if *column < left_names.len() {
                        &left[*column]
                    } else {
                        &right[*column - left_names.len()]
                    };
                    let value = if matches!(value, typed::Cell::Null) {
                        fallback.and_then(|index| {
                            index.checked_sub(left_names.len())
                                .and_then(|index| right.get(index))
                        }).unwrap_or(value)
                    } else { value };
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
    let keep = retained_build_columns(right_names, right_keys, selected, null_filter, left_names.len());
    let right_rows = read_projected_build_rows(
        right_path,
        right_names,
        &keep,
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
    let mut retained = if options.materialize_rows {
        (0..right_names.len()).collect::<Vec<_>>()
    } else {
        right_keys.to_vec()
    };
    let left_width = left_rows.first().map_or(0, Vec::len);
    if let Some((column, _)) = options.null_filter {
        if let Some(local) = column.checked_sub(left_width)
            .filter(|&local| local < right_names.len()) {
            retained.push(local);
        }
    }
    retained.sort_unstable();
    retained.dedup();
    if retained.is_empty() {
        retained.push(0);
    }
    let selected = retained.iter().map(|&index| right_names[index].clone()).collect::<Vec<_>>();
    let key_indices = right_keys.iter().map(|index| {
        retained.binary_search(index).expect("join key retained")
    }).collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(right_path, &selected)?;
    let mut matches = 0usize;
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
            if batch_key_into(&batch, index, &key_indices, &mut probe_key) {
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
                                typed::is_null_at(
                                    &batch[retained.binary_search(&(column - left_width)).expect("filter retained")],
                                    index,
                                )
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
    let keep = retained_build_columns(left_names, left_keys, selected, null_filter, 0);
    let left_rows = read_projected_build_rows(
        left_path,
        left_names,
        &keep,
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


struct OrderedJoinRow {
    values: Vec<typed::Cell>,
    keys: Vec<typed::Cell>,
    directions: Arc<[(bool, Option<bool>)]>,
    sequence: usize,
    retained_bytes: usize,
}

impl PartialEq for OrderedJoinRow {
    fn eq(&self, other: &Self) -> bool { self.cmp(other) == Ordering::Equal }
}
impl Eq for OrderedJoinRow {}
impl PartialOrd for OrderedJoinRow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for OrderedJoinRow {
    fn cmp(&self, other: &Self) -> Ordering {
        for ((left, right), &(descending, nulls_first)) in self.keys.iter()
            .zip(&other.keys).zip(self.directions.iter()) {
            let ordering = typed::compare_order_cells(left, right, descending, nulls_first);
            if ordering != Ordering::Equal { return ordering; }
        }
        self.sequence.cmp(&other.sequence)
    }
}

fn ordered_join_retained_bytes(cells: &[typed::Cell]) -> Result<usize, ExecutionError> {
    cells.iter().try_fold(
        std::mem::size_of_val(cells),
        |total, cell| total.checked_add(match cell {
            typed::Cell::Text(bytes) => bytes.len(),
            typed::Cell::Uuid(value) => value.len(),
            _ => 0,
        }).ok_or_else(|| invalid("ordered JOIN output size overflow"))
    )
}

/// Scan an arbitrarily large fact side while retaining only the requested
/// globally ordered page. The bounded dimension hash build is unchanged.
fn execute_streaming_ordered_inner<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    columns: InnerJoinColumns<'_>,
    order: &[(usize, bool, Option<bool>)],
    requested: usize,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let InnerJoinColumns {
        left_names, right_names, selected, left_keys, right_keys, null_filter
    } = columns;
    let right_metadata = inspect_query_source(right_path)?;
    let mut keep = retained_build_columns(
        right_names, right_keys, selected, null_filter, left_names.len(),
    );
    // An ORDER BY column can be absent from SELECT but is still needed
    // when comparing the streamed joined rows.
    for &(index, _, _) in order {
        if let Some(local) = index.checked_sub(left_names.len())
            .filter(|&index| index < right_names.len()) {
            keep.push(local);
        }
    }
    keep.sort_unstable();
    keep.dedup();
    let right_rows = read_projected_build_rows(
        right_path, right_names, &keep, right_metadata.rows,
        MAX_STREAM_BUILD_ROWS, MAX_STREAM_BUILD_BYTES,
    )?;
    let mut hash = HashMap::<Vec<typed::Key>, Bucket>::with_capacity(right_rows.len());
    for (index, row) in right_rows.iter().enumerate() {
        if let Some(key) = row_key(row, right_keys) {
            if query.any {
                hash.entry(key).or_insert(Bucket::One(index));
            } else {
                hash.entry(key)
                    .and_modify(|bucket| bucket.push(index))
                    .or_insert(Bucket::One(index));
            }
        }
    }
    let mut heap = BinaryHeap::<OrderedJoinRow>::with_capacity(requested.min(1024));
    let directions: Arc<[(bool, Option<bool>)]> =
        order.iter().map(|(_, desc, nulls)| (*desc, *nulls)).collect::<Vec<_>>().into();
    let mut retained = 0usize;
    let mut sequence = 0usize;
    let width = left_names.len();
    probe_inner(
        left_path, left_names, left_keys, &right_rows, &hash,
        ProbeOptions {
            materialize_rows: true,
            materialize_after: 0,
            // A small ordered result still needs to inspect every qualifying
            // match, without the unordered 131072-output limit.
            stop_after: Some(usize::MAX),
            any: query.any,
            null_filter,
        },
        |left, right| {
            let at = |index: usize| -> &typed::Cell {
                if index < width { &left[index] } else { &right[index - width] }
            };
            let keys: Vec<typed::Cell> = order.iter().map(|(index, _, _)| at(*index).clone()).collect();
            let values: Vec<typed::Cell> = selected
                .iter().map(|(index, _, _)| at(*index).clone()).collect();
            let bytes = ordered_join_retained_bytes(&keys)?
                .checked_add(ordered_join_retained_bytes(&values)?)
                .ok_or_else(|| invalid("ordered JOIN output size overflow"))?;
            let candidate = OrderedJoinRow {
                values, keys, directions: directions.clone(), sequence,
                retained_bytes: bytes,
            };
            sequence = sequence.checked_add(1)
                .ok_or_else(|| invalid("ordered JOIN row count overflow"))?;
            if heap.len() == requested &&
                heap.peek().is_some_and(|worst| candidate >= *worst) {
                return Ok(());
            }
            let removed = if heap.len() == requested {
                heap.peek().map_or(0, |row| row.retained_bytes)
            } else { 0 };
            let total = retained.checked_sub(removed)
                .and_then(|value| value.checked_add(bytes))
                .ok_or_else(|| invalid("ordered JOIN output size overflow"))?;
            if total > MAX_STREAM_BUILD_BYTES {
                return Err(invalid("ordered JOIN exceeds its retained-row memory budget"));
            }
            retained = total;
            if heap.len() < requested {
                heap.push(candidate);
            } else {
                *heap.peek_mut().expect("nonempty ordered JOIN heap") = candidate;
            }
            Ok(())
        },
    )?;
    let header = selected.iter().map(|(_, name, _)| name.as_str()).collect::<Vec<_>>().join("\t");
    writeln!(output, "{header}")?;
    for row in heap.into_sorted_vec().into_iter().skip(query.offset) {
        for (index, value) in row.values.iter().enumerate() {
            if index != 0 { output.write_all(b"\t")?; }
            typed::write_value(output, value)?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}


fn execute_streaming_inner_count<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    columns: InnerJoinColumns<'_>,
    left_count: u64,
    right_count: u64,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let InnerJoinColumns {
        left_names, right_names, left_keys, right_keys, null_filter, ..
    } = columns;
    if query.limit == Some(0) || query.offset > 0 {
        output.write_all(b"count\n")?;
        return Ok(());
    }
    if left_count == 0 || right_count == 0 {
        output.write_all(b"count\n0\n")?;
        return Ok(());
    }
    let count = if right_count <= MAX_STREAM_BUILD_ROWS &&
        (right_count <= left_count || left_count > MAX_STREAM_BUILD_ROWS) {
        let keep = count_build_columns(
            right_keys, null_filter, left_names.len(), right_names.len(),
        );
        let right_rows = read_projected_build_rows(
            right_path, right_names, &keep, right_count,
            MAX_STREAM_BUILD_ROWS, MAX_STREAM_BUILD_BYTES,
        )?;
        let mut hash = HashMap::<Vec<typed::Key>, Bucket>::with_capacity(right_rows.len());
        for (index, row) in right_rows.iter().enumerate() {
            if let Some(key) = row_key(row, right_keys) {
                if query.any {
                    hash.entry(key).or_insert(Bucket::One(index));
                } else {
                    hash.entry(key)
                        .and_modify(|bucket| bucket.push(index))
                        .or_insert(Bucket::One(index));
                }
            }
        }
        probe_inner(
            left_path, left_names, left_keys, &right_rows, &hash,
            ProbeOptions {
                materialize_rows: false, materialize_after: 0,
                stop_after: Some(usize::MAX), any: query.any, null_filter,
            }, |_, _| Ok(())
        )?
    } else {
        let keep = count_build_columns(
            left_keys, null_filter, 0, left_names.len(),
        );
        let left_rows = read_projected_build_rows(
            left_path, left_names, &keep, left_count,
            MAX_STREAM_BUILD_ROWS, MAX_STREAM_BUILD_BYTES,
        )?;
        let mut hash = HashMap::<Vec<typed::Key>, Bucket>::with_capacity(left_rows.len());
        for (index, row) in left_rows.iter().enumerate() {
            if let Some(key) = row_key(row, left_keys) {
                hash.entry(key)
                    .and_modify(|bucket| bucket.push(index))
                    .or_insert(Bucket::One(index));
            }
        }
        probe_right(
            right_path, right_names, right_keys, &left_rows, &hash,
            ProbeOptions {
                materialize_rows: false, materialize_after: 0,
                stop_after: Some(usize::MAX), any: query.any, null_filter,
            }, |_, _| Ok(())
        )?
    };
    write!(output, "count\n{count}\n")?;
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

    if query.grouped_count.is_some() {
        return grouped::execute(
            query,
            left_path,
            right_path,
            &left_names,
            &right_names,
            &left_keys,
            &right_keys,
            null_filter,
            output,
        );
    }

    if query.aggregate == Some(JoinAggregate::CountStar) {
        return execute_streaming_inner_count(
            query, left_path, right_path,
            InnerJoinColumns {
                left_names: &left_names, right_names: &right_names,
                selected: &selected, left_keys: &left_keys, right_keys: &right_keys,
                null_filter,
            },
            left_meta.rows, right_meta.rows, output
        );
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

    if query.kind == JoinType::Right && order.is_empty()
        && left_meta.rows <= MAX_STREAM_BUILD_ROWS
        && right_meta.rows > MAX_INPUT_ROWS
    {
        return execute_streaming_right_outer(
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

    if query.kind == JoinType::Inner
        && !order.is_empty()
        && left_meta.rows > MAX_INPUT_ROWS
        && right_meta.rows <= MAX_STREAM_BUILD_ROWS
    {
        if let Some(requested) = query.limit
            .and_then(|limit| query.offset.checked_add(limit))
            .filter(|&requested| requested <= MAX_RESULT_ROWS) {
            return execute_streaming_ordered_inner(
                query, left_path, right_path,
                InnerJoinColumns {
                    left_names: &left_names,
                    right_names: &right_names,
                    selected: &selected,
                    left_keys: &left_keys,
                    right_keys: &right_keys,
                    null_filter,
                },
                &order, requested, output
            );
        }
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
    fn streaming_right_preserves_unmatched_rows_and_key_fallbacks() {
        use rusthouse_storage::{Column, write_wide_part};
        use std::fs;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rusthouse-right-stream-test-{}-{}",
            std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let right = root.join("right");
        let schema = [Column { name: "id".into(), data_type: "UInt64".into() }];
        write_wide_part(&right, &schema, &[
            vec![Some("1".into())], vec![Some("3".into())],
            vec![Some("2".into())],
        ]).unwrap();
        let left_rows = vec![
            vec![typed::Cell::Numeric(Number::Unsigned(1))],
            vec![typed::Cell::Numeric(Number::Unsigned(1))],
        ];
        let mut hash = HashMap::new();
        hash.insert(vec![typed::Cell::Numeric(Number::Unsigned(1)).key()],
            Bucket::Many(vec![0, 1]));
        let mut seen = Vec::new();
        let count = probe_right_outer(
            &right, &["id".into()], &[0], &left_rows, 1, &hash,
            ProbeOptions { materialize_rows: true, materialize_after: 0,
                stop_after: None, any: false, null_filter: None },
            |left, right| {
                let id = match &right[0] {
                    typed::Cell::Numeric(Number::Unsigned(id)) => *id,
                    _ => panic!("expected right id"),
                };
                seen.push((id, matches!(&left[0], typed::Cell::Null)));
                Ok(())
            },
        ).unwrap();
        assert_eq!(count, 4);
        assert_eq!(seen, vec![(1, false), (1, false), (3, true), (2, true)]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn projected_hash_build_only_reads_referenced_wide_columns() {
        use rusthouse_storage::{Column, write_wide_part};
        use std::fs;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rusthouse-join-build-pruning-{}-{}",
            std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let names = vec!["id".into(), "unused".into(), "label".into()];
        let schema = vec![
            Column { name: names[0].clone(), data_type: "UInt64".into() },
            Column { name: names[1].clone(), data_type: "String".into() },
            Column { name: names[2].clone(), data_type: "String".into() },
        ];
        write_wide_part(
            &root, &schema, &[
                vec![Some("5".into()), Some("unneeded-heavy-payload".repeat(100)), Some("ok".into())],
            ],
        ).unwrap();
        let selected = vec![(3, "fact.id".into(), None), (5, "dimension.label".into(), None)];
        let keep = retained_build_columns(&names, &[0], &selected, None, 3);
        assert_eq!(keep, vec![0, 2]);
        let rows = read_projected_build_rows(&root, &names, &keep, 1, 10, 4_096).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0][1], typed::Cell::Null));
        assert!(matches!(&rows[0][2], typed::Cell::Text(value) if value == b"ok"));
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

#[cfg(test)]
mod projected_count_build_tests {
    use super::*;

    #[test]
    fn keeps_keys_filters_and_cross_row_counter_only() {
        assert_eq!(count_build_columns(&[2, 0, 2], Some((4, true)), 3, 5), vec![0, 1, 2]);
        assert_eq!(count_build_columns(&[0], Some((7, false)), 3, 5), vec![0, 4]);
        assert_eq!(count_build_columns(&[], None, 0, 4), vec![0]);
        assert!(count_build_columns(&[], None, 0, 0).is_empty());
    }
}
