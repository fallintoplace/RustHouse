//! In-memory, bounded equi-join over validated immutable typed snapshots.

mod aggregate;
mod build_rows;
mod comparison_filter;
mod count_column;
mod grouped;
mod keys;
mod runner;
pub use runner::execute_join;
pub(super) use self::build_rows::*;

use self::keys::*;

use super::{ExecutionError, Number, NumberKey, typed};
use crate::{JoinAggregate, JoinColumn, JoinSelect, JoinType};
use rusthouse_storage::{PartMetadata, TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, hash_map::Entry};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

// Ordered and outer joins retain both inputs, bounded by row count and bytes.
pub(super) const MAX_INPUT_ROWS: u64 = crate::join::MAX_BUFFERED_JOIN_ROWS;
pub(super) const MAX_RESULT_ROWS: usize = crate::join::MAX_JOIN_RESULT_ROWS;
// Unordered INNER joins can build a larger hash side than sorted/outer joins.
// Bound retained cells as well as row count to protect against wide String rows.
const MAX_STREAM_BUILD_ROWS: u64 = crate::join::MAX_STREAM_JOIN_BUILD_ROWS;
pub(super) const MAX_STREAM_BUILD_BYTES: usize = crate::join::MAX_JOIN_BUILD_BYTES;
/// Integers of different widths or signedness can be equi-joined exactly
/// through the canonical 128-bit hash key; floats remain strict by type.
pub(super) fn compatible_join_types(left: &str, right: &str) -> bool {
    let (left, right) = (base_type(left), base_type(right));
    if left == right {
        return true;
    }
    let integer = |kind: &str| {
        matches!(
            kind,
            "UInt8"
                | "UInt16"
                | "UInt32"
                | "UInt64"
                | "UInt128"
                | "Int8"
                | "Int16"
                | "Int32"
                | "Int64"
                | "Int128"
        )
    };
    integer(left) && integer(right)
}

type Pair = (Option<usize>, Option<usize>);

// Distinct join keys can be much fewer than build rows on skewed dimensions.
// Grow the table with observed cardinality instead of preallocating for all rows.
fn initial_hash_capacity(rows: usize) -> usize {
    rows.min(4096)
}

// A large input row count is not proof that the hash build fits the byte budget.
const INITIAL_JOIN_ROW_CAPACITY: u64 = 4_096;

fn initial_join_row_capacity(expected_rows: u64) -> usize {
    expected_rows.min(INITIAL_JOIN_ROW_CAPACITY) as usize
}

#[cfg(test)]
pub(super) fn multi_join_row_key(
    row: &[typed::Cell],
    indices: &[usize],
) -> Option<Vec<typed::Key>> {
    row_key(row, indices)
}

pub(super) fn multi_join_row_key_into(
    row: &[typed::Cell],
    indices: &[usize],
    result: &mut Vec<typed::Key>,
) -> bool {
    row_key_into(row, indices, result)
}

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

/// Avoid allocating a heap Vec for the common unique JOIN build key.
/// Promote to a boxed Vec only when the build side contains duplicate keys.
/// The extra box keeps the larger Vec header out of every unique-key slot.
#[allow(clippy::box_collection)]
pub(super) enum Bucket {
    One(usize),
    Many(Box<Vec<usize>>),
}

impl Bucket {
    pub(super) fn push(&mut self, value: usize) {
        match self {
            Self::One(first) => *self = Self::Many(Box::new(vec![*first, value])),
            Self::Many(values) => values.push(value),
        }
    }

    pub(super) fn iter(&self) -> std::slice::Iter<'_, usize> {
        match self {
            Self::One(value) => std::slice::from_ref(value).iter(),
            Self::Many(values) => values.iter(),
        }
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Many(values) => values.len(),
        }
    }
}

/// Store single-column and nonnegative numeric pair keys without key vectors.
pub(super) enum JoinHash {
    SingleNumeric(HashMap<NumberKey, Bucket>),
    SingleOther(HashMap<typed::Key, Bucket>),
    CompositeUnsignedPair(HashMap<(u128, u128), Bucket>),
    Composite(HashMap<Vec<typed::Key>, Bucket>),
}

impl JoinHash {
    pub(super) fn with_capacity(key_count: usize, capacity: usize, single_numeric: bool) -> Self {
        if key_count == 1 {
            if single_numeric {
                Self::SingleNumeric(HashMap::with_capacity(capacity))
            } else {
                Self::SingleOther(HashMap::with_capacity(capacity))
            }
        } else {
            Self::Composite(HashMap::with_capacity(capacity))
        }
    }

    pub(super) fn with_unsigned_pair_capacity(capacity: usize) -> Self {
        Self::CompositeUnsignedPair(HashMap::with_capacity(capacity))
    }

    pub(super) fn lookup(&self, key: &[typed::Key]) -> Option<&Bucket> {
        match self {
            Self::SingleNumeric(hash) if key.len() == 1 => match &key[0] {
                typed::Key::Numeric(number) => hash.get(number),
                _ => None,
            },
            Self::SingleNumeric(_) => None,
            Self::SingleOther(hash) if key.len() == 1 => hash.get(&key[0]),
            Self::SingleOther(_) => None,
            Self::CompositeUnsignedPair(hash) => {
                let [
                    typed::Key::Numeric(NumberKey::Unsigned(first)),
                    typed::Key::Numeric(NumberKey::Unsigned(second)),
                ] = key
                else {
                    return None;
                };
                hash.get(&(*first, *second))
            }
            Self::Composite(hash) => hash.get(key),
        }
    }

    pub(super) fn lookup_mut(&mut self, key: &[typed::Key]) -> Option<&mut Bucket> {
        match self {
            Self::SingleNumeric(hash) if key.len() == 1 => match &key[0] {
                typed::Key::Numeric(number) => hash.get_mut(number),
                _ => None,
            },
            Self::SingleNumeric(_) => None,
            Self::SingleOther(hash) if key.len() == 1 => hash.get_mut(&key[0]),
            Self::SingleOther(_) => None,
            Self::CompositeUnsignedPair(hash) => {
                let [
                    typed::Key::Numeric(NumberKey::Unsigned(first)),
                    typed::Key::Numeric(NumberKey::Unsigned(second)),
                ] = key
                else {
                    return None;
                };
                hash.get_mut(&(*first, *second))
            }
            Self::Composite(hash) => hash.get_mut(key),
        }
    }

    pub(super) fn insert_new(&mut self, key: &[typed::Key], index: usize) {
        match self {
            Self::SingleNumeric(hash) => {
                debug_assert_eq!(key.len(), 1);
                let typed::Key::Numeric(number) = &key[0] else {
                    unreachable!("numeric JOIN key changed type while building");
                };
                hash.insert(*number, Bucket::One(index));
            }
            Self::SingleOther(hash) => {
                debug_assert_eq!(key.len(), 1);
                hash.insert(key[0].clone(), Bucket::One(index));
            }
            Self::CompositeUnsignedPair(hash) => {
                let [
                    typed::Key::Numeric(NumberKey::Unsigned(first)),
                    typed::Key::Numeric(NumberKey::Unsigned(second)),
                ] = key
                else {
                    unreachable!("unsigned pair JOIN keys changed type while building");
                };
                hash.insert((*first, *second), Bucket::One(index));
            }
            Self::Composite(hash) => {
                hash.insert(key.to_vec(), Bucket::One(index));
            }
        }
    }

    fn merge_from(&mut self, other: Self, any: bool) {
        match (self, other) {
            (Self::SingleNumeric(target), Self::SingleNumeric(source)) => {
                merge_bucket_maps(target, source, any);
            }
            (Self::SingleOther(target), Self::SingleOther(source)) => {
                merge_bucket_maps(target, source, any);
            }
            (Self::CompositeUnsignedPair(target), Self::CompositeUnsignedPair(source)) => {
                merge_bucket_maps(target, source, any);
            }
            (Self::Composite(target), Self::Composite(source)) => {
                merge_bucket_maps(target, source, any);
            }
            _ => unreachable!("join hash key arity changed while merging"),
        }
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::SingleNumeric(hash) => hash.len(),
            Self::SingleOther(hash) => hash.len(),
            Self::CompositeUnsignedPair(hash) => hash.len(),
            Self::Composite(hash) => hash.len(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn total_bucket_items(&self) -> usize {
        match self {
            Self::SingleNumeric(hash) => hash.values().map(Bucket::len).sum(),
            Self::SingleOther(hash) => hash.values().map(Bucket::len).sum(),
            Self::CompositeUnsignedPair(hash) => hash.values().map(Bucket::len).sum(),
            Self::Composite(hash) => hash.values().map(Bucket::len).sum(),
        }
    }
}

pub(super) fn single_numeric_join_key(rows: &[Vec<typed::Cell>], keys: &[usize]) -> bool {
    if keys.len() != 1 {
        return false;
    }
    // A typed column has one value type, so the first non-null key selects the map.
    rows.iter()
        .find_map(|row| match &row[keys[0]] {
            typed::Cell::Numeric(_) => Some(true),
            typed::Cell::Null => None,
            _ => Some(false),
        })
        .unwrap_or(false)
}

pub(super) fn unsigned_composite_join_key(rows: &[Vec<typed::Cell>], keys: &[usize]) -> bool {
    keys.len() == 2
        && keys.iter().all(|&index| {
            rows.iter()
                .find_map(|row| match row.get(index) {
                    Some(typed::Cell::Numeric(Number::Unsigned(_)))
                    | Some(typed::Cell::Numeric(Number::Signed(0..=i128::MAX))) => Some(true),
                    Some(typed::Cell::Numeric(_)) => Some(false),
                    Some(typed::Cell::Null) => None,
                    Some(_) | None => Some(false),
                })
                .unwrap_or(false)
        })
}

trait JoinLookup {
    fn lookup(&self, key: &[typed::Key]) -> Option<&Bucket>;
    fn is_empty(&self) -> bool;
    fn total_bucket_items(&self) -> usize;
}

impl JoinLookup for JoinHash {
    fn lookup(&self, key: &[typed::Key]) -> Option<&Bucket> {
        JoinHash::lookup(self, key)
    }

    fn is_empty(&self) -> bool {
        JoinHash::is_empty(self)
    }

    fn total_bucket_items(&self) -> usize {
        JoinHash::total_bucket_items(self)
    }
}

impl JoinLookup for HashMap<Vec<typed::Key>, Bucket> {
    fn lookup(&self, key: &[typed::Key]) -> Option<&Bucket> {
        self.get(key)
    }

    fn is_empty(&self) -> bool {
        HashMap::is_empty(self)
    }

    fn total_bucket_items(&self) -> usize {
        self.values().map(Bucket::len).sum()
    }
}

fn merge_bucket_maps<K>(target: &mut HashMap<K, Bucket>, source: HashMap<K, Bucket>, any: bool)
where
    K: Eq + std::hash::Hash,
{
    for (key, bucket) in source {
        match target.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(bucket);
            }
            Entry::Occupied(mut entry) if !any => {
                for &index in bucket.iter() {
                    entry.get_mut().push(index);
                }
            }
            Entry::Occupied(_) => {}
        }
    }
}

/// Duplicate build keys are not needed when ANY joins select one right row
/// for each left row. Keep the first row to match existing stream behavior.
#[cfg(test)]
fn insert_build_key(
    hash: &mut HashMap<Vec<typed::Key>, Bucket>,
    key: Vec<typed::Key>,
    index: usize,
    any: bool,
) {
    if any {
        hash.entry(key).or_insert(Bucket::One(index));
    } else {
        hash.entry(key)
            .and_modify(|bucket| bucket.push(index))
            .or_insert(Bucket::One(index));
    }
}

/// Reuse the probe-style key buffer while building a hash table. Skewed
/// dimensions can have thousands of duplicate keys: only unique keys need
/// an owned allocation retained in the table.
fn insert_build_row(
    hash: &mut JoinHash,
    row: &[typed::Cell],
    indices: &[usize],
    scratch: &mut Vec<typed::Key>,
    index: usize,
    any: bool,
) {
    if !row_key_into(row, indices, scratch) {
        return;
    }
    if let Some(bucket) = hash.lookup_mut(scratch.as_slice()) {
        if !any {
            bucket.push(index);
        }
    } else {
        hash.insert_new(scratch, index);
    }
}

/// Build independent contiguous JOIN hash partitions and merge in source
/// order. This preserves the first row used by ANY joins and the duplicate
/// match order required by ordinary inner joins.
fn build_join_hash(
    rows: &[Vec<typed::Cell>],
    keys: &[usize],
    any: bool,
    workers: usize,
) -> Result<JoinHash, ExecutionError> {
    const MIN_PARALLEL_ROWS: usize = 16_384;
    let single_numeric = single_numeric_join_key(rows, keys);
    if rows.len() < MIN_PARALLEL_ROWS || workers < 2 {
        let mut hash = JoinHash::with_capacity(
            keys.len(),
            initial_hash_capacity(rows.len()),
            single_numeric,
        );
        let mut scratch = Vec::with_capacity(keys.len());
        for (index, row) in rows.iter().enumerate() {
            insert_build_row(&mut hash, row, keys, &mut scratch, index, any);
        }
        return Ok(hash);
    }
    let chunk_size = rows.len().div_ceil(workers);
    let partials = std::thread::scope(|scope| -> Result<_, ExecutionError> {
        let handles = rows
            .chunks(chunk_size)
            .enumerate()
            .map(|(chunk_index, chunk)| {
                let start = chunk_index * chunk_size;
                std::thread::Builder::new()
                    .spawn_scoped(scope, move || {
                        let mut partial = JoinHash::with_capacity(
                            keys.len(),
                            initial_hash_capacity(chunk.len()),
                            single_numeric,
                        );
                        let mut scratch = Vec::with_capacity(keys.len());
                        for (local, row) in chunk.iter().enumerate() {
                            insert_build_row(
                                &mut partial,
                                row,
                                keys,
                                &mut scratch,
                                start + local,
                                any,
                            );
                        }
                        partial
                    })
                    .map_err(|error| {
                        ExecutionError::Invalid(format!(
                            "cannot start parallel JOIN build worker: {error}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        handles
            .into_iter()
            .map(|handle| {
                handle.join().map_err(|_| {
                    ExecutionError::Invalid("parallel JOIN build worker panicked".into())
                })
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    let mut pieces = partials.into_iter();
    let mut hash = pieces
        .next()
        .expect("a parallel JOIN build has at least one partition");
    for piece in pieces {
        hash.merge_from(piece, any);
    }
    Ok(hash)
}

struct ProbeOptions {
    materialize_rows: bool,
    materialize_after: usize,
    stop_after: Option<usize>,
    any: bool,
    null_filter: Option<(usize, bool)>,
}

/// Preserve the original whole-row probe API for paths that need every
/// source column. Selective callers use probe_inner_projected instead.
fn probe_inner<H, F>(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    hash: &H,
    options: ProbeOptions,
    emit: F,
) -> Result<usize, ExecutionError>
where
    H: JoinLookup,
    F: FnMut(&[typed::Cell], &[typed::Cell]) -> Result<(), ExecutionError>,
{
    probe_inner_projected(
        left_path, left_names, left_keys, right_rows, hash, options, None, emit,
    )
}

/// Reconstruct original source positions from the compact typed scan.
fn reconstruct_join_projection(
    batch: &[TypedColumn],
    row: usize,
    retained: &[usize],
    original_width: usize,
    projected_mask: &[bool],
) -> Vec<typed::Cell> {
    let mut cells = vec![typed::Cell::Null; original_width];
    for (&original, column) in retained.iter().zip(batch.iter()) {
        if projected_mask[original] {
            cells[original] = typed::cell(column, row);
        }
    }
    cells
}

/// Scan the probe side without retaining its rows or joined result pairs.
#[allow(clippy::too_many_arguments)]
fn probe_inner_projected<H, F>(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    hash: &H,
    options: ProbeOptions,
    projected: Option<&[usize]>,
    mut emit: F,
) -> Result<usize, ExecutionError>
where
    H: JoinLookup,
    F: FnMut(&[typed::Cell], &[typed::Cell]) -> Result<(), ExecutionError>,
{
    if options.stop_after == Some(0) || hash.is_empty() {
        return Ok(0);
    }
    // A COUNT or preflight probe needs join keys and filters only. Keep
    // original slot positions when actual output rows must be materialized.
    let mut retained = if options.materialize_rows {
        if let Some(columns) = projected {
            let mut required = left_keys.to_vec();
            required.extend_from_slice(columns);
            required
        } else {
            (0..left_names.len()).collect::<Vec<_>>()
        }
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
    let projected_mask = projected.map(|columns| {
        let mut mask = vec![false; left_names.len()];
        for &index in columns {
            mask[index] = true;
        }
        mask
    });
    let selected = retained
        .iter()
        .map(|&index| left_names[index].clone())
        .collect::<Vec<_>>();
    let key_indices = left_keys
        .iter()
        .map(|index| retained.binary_search(index).expect("join key retained"))
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(left_path, &selected)?;
    let mut matches = 0usize;
    let mut probe_key = Vec::with_capacity(left_keys.len());
    'scan: while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            if batch_key_into(&batch, index, &key_indices, &mut probe_key) {
                if let Some(right_indices) = hash.lookup(probe_key.as_slice()) {
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
                                    &batch
                                        [retained.binary_search(&column).expect("filter retained")],
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
                                        if let Some(mask) = projected_mask.as_deref() {
                                            reconstruct_join_projection(
                                                &batch,
                                                index,
                                                &retained,
                                                left_names.len(),
                                                mask,
                                            )
                                        } else {
                                            batch
                                                .iter()
                                                .map(|column| typed::cell(column, index))
                                                .collect()
                                        }
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
#[allow(clippy::too_many_arguments)]
fn probe_right_outer<H, F>(
    right_path: &Path,
    right_names: &[String],
    right_keys: &[usize],
    left_rows: &[Vec<typed::Cell>],
    left_width: usize,
    hash: &H,
    options: ProbeOptions,
    mut emit: F,
) -> Result<usize, ExecutionError>
where
    H: JoinLookup,
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
    if let Some((column, _)) = options.null_filter {
        if let Some(local) = column
            .checked_sub(left_width)
            .filter(|&local| local < right_names.len())
        {
            retained.push(local);
        }
    }
    retained.sort_unstable();
    retained.dedup();
    if retained.is_empty() {
        retained.push(0);
    }
    let selected = retained
        .iter()
        .map(|&index| right_names[index].clone())
        .collect::<Vec<_>>();
    let key_indices = right_keys
        .iter()
        .map(|index| retained.binary_search(index).expect("join key retained"))
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(right_path, &selected)?;
    let null_left = vec![typed::Cell::Null; left_width];
    let mut total = 0usize;
    let mut probe_key = Vec::with_capacity(right_keys.len());
    'scan: while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            let candidates = if batch_key_into(&batch, row, &key_indices, &mut probe_key) {
                hash.lookup(probe_key.as_slice())
            } else {
                None
            };
            let matched = candidates.is_some();
            let mut right_row: Option<Vec<typed::Cell>> = None;
            let matches = candidates.into_iter().flat_map(|bucket| {
                bucket
                    .iter()
                    .take(if options.any { 1 } else { usize::MAX })
                    .map(Some)
            });
            for left_index in matches.chain((!matched).then_some(None)) {
                let left =
                    left_index.map_or(null_left.as_slice(), |&index| left_rows[index].as_slice());
                if let Some((column, not_null)) = options.null_filter {
                    let is_null = if column < left_width {
                        matches!(&left[column], typed::Cell::Null)
                    } else {
                        typed::is_null_at(
                            &batch[retained
                                .binary_search(&(column - left_width))
                                .expect("filter retained")],
                            row,
                        )
                    };
                    if is_null == not_null {
                        continue;
                    }
                }
                if options.stop_after.is_some_and(|end| total >= end) {
                    break 'scan;
                }
                if options.stop_after.is_none() && total >= MAX_RESULT_ROWS {
                    return Err(invalid("JOIN produces more than 131072 rows"));
                }
                let right = if options.materialize_rows && total >= options.materialize_after {
                    right_row
                        .get_or_insert_with(|| {
                            batch
                                .iter()
                                .map(|column| typed::cell(column, row))
                                .collect()
                        })
                        .as_slice()
                } else {
                    &[]
                };
                emit(left, right)?;
                total += 1;
            }
        }
    }
    Ok(total)
}

/// Keep JOIN projection formatting tied to the underlying column types.
fn join_output_types(
    left_path: &Path,
    right_path: &Path,
    selected: &[(usize, String, Option<usize>)],
) -> Result<Vec<String>, ExecutionError> {
    let left = inspect_query_source(left_path)?;
    let right = inspect_query_source(right_path)?;
    let types = left
        .columns
        .iter()
        .chain(&right.columns)
        .map(|column| column.data_type.as_str())
        .collect::<Vec<_>>();
    selected
        .iter()
        .map(|(index, _, _)| {
            types
                .get(*index)
                .map(|data_type| (*data_type).to_owned())
                .ok_or_else(|| invalid("unknown JOIN projection type"))
        })
        .collect()
}

fn execute_streaming_right_outer<W: Write>(
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
    let projection_types = join_output_types(left_path, right_path, selected)?;
    let left_meta = inspect_query_source(left_path)?;
    let keep = retained_build_columns(left_names, left_keys, selected, null_filter, 0);
    let left_rows = read_projected_build_rows_using_metadata(
        left_path,
        left_names,
        &keep,
        left_meta.rows,
        MAX_STREAM_BUILD_ROWS,
        MAX_STREAM_BUILD_BYTES,
        Some(&left_meta),
    )?;
    let mut hash =
        HashMap::<Vec<typed::Key>, Bucket>::with_capacity(initial_hash_capacity(left_rows.len()));
    for (index, row) in left_rows.iter().enumerate() {
        if let Some(key) = row_key(row, left_keys) {
            if query.any {
                hash.entry(key).or_insert(Bucket::One(index));
            } else {
                hash.entry(key)
                    .and_modify(|bucket| bucket.push(index))
                    .or_insert(Bucket::One(index));
            }
        }
    }
    let stop_after = query
        .limit
        .filter(|&limit| limit <= MAX_RESULT_ROWS)
        .and_then(|limit| query.offset.checked_add(limit));
    let preflight = probe_right_outer(
        right_path,
        right_names,
        right_keys,
        &left_rows,
        left_names.len(),
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
    let header = selected_header(selected);
    writeln!(output, "{header}")?;
    if preflight <= query.offset {
        return Ok(());
    }
    let end = query
        .offset
        .saturating_add(query.limit.unwrap_or(usize::MAX));
    let mut seen = 0usize;
    probe_right_outer(
        right_path,
        right_names,
        right_keys,
        &left_rows,
        left_names.len(),
        &hash,
        ProbeOptions {
            materialize_rows: selected
                .iter()
                .any(|(column, _, fallback)| *column >= left_names.len() || fallback.is_some()),
            materialize_after: query.offset,
            stop_after,
            any: query.any,
            null_filter,
        },
        |left, right| {
            if seen >= query.offset && seen < end {
                for (pos, (column, _, fallback)) in selected.iter().enumerate() {
                    if pos > 0 {
                        output.write_all(b"\t")?;
                    }
                    let value = if *column < left_names.len() {
                        &left[*column]
                    } else {
                        &right[*column - left_names.len()]
                    };
                    let value = if matches!(value, typed::Cell::Null) {
                        fallback
                            .and_then(|index| {
                                index
                                    .checked_sub(left_names.len())
                                    .and_then(|index| right.get(index))
                            })
                            .unwrap_or(value)
                    } else {
                        value
                    };
                    typed::write_typed_value(output, value, &projection_types[pos])?;
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
    let projection_types = join_output_types(left_path, right_path, selected)?;
    // The probe side is scanned in batches and never retained. Keep the
    // hash-build and matched-output limits, not an artificial probe-row cap.
    let right_meta = inspect_query_source(right_path)?;
    let keep = retained_build_columns(
        right_names,
        right_keys,
        selected,
        null_filter,
        left_names.len(),
    );
    let right_rows = read_projected_build_rows_using_metadata(
        right_path,
        right_names,
        &keep,
        right_meta.rows,
        MAX_STREAM_BUILD_ROWS,
        MAX_STREAM_BUILD_BYTES,
        Some(&right_meta),
    )?;
    let workers = super::query_workers(4);
    let hash = build_join_hash(&right_rows, right_keys, query.any, workers)?;
    if hash.is_empty() {
        let header = selected_header(selected);
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

    let header = selected_header(selected);
    writeln!(output, "{header}")?;
    if preflight_matches <= query.offset {
        return Ok(());
    }

    let left_width = left_names.len();
    let end = query
        .offset
        .saturating_add(query.limit.unwrap_or(usize::MAX));
    let mut position = 0usize;
    // JOIN projections write separators and scalar cells independently.
    // Buffer the output once for large streamed matches, while letting
    // BufWriter forward unusually large String cells without copying them.
    let projected_left = selected
        .iter()
        .filter_map(|(index, _, _)| (*index < left_width).then_some(*index))
        .collect::<Vec<_>>();
    let mut row_output = std::io::BufWriter::with_capacity(64 * 1024, output);
    probe_inner_projected(
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
        Some(&projected_left),
        |left, right| {
            if position >= query.offset && position < end {
                for (index, (column, _, _)) in selected.iter().enumerate() {
                    if index != 0 {
                        row_output.write_all(b"\t")?;
                    }
                    let cell = if *column < left_width {
                        &left[*column]
                    } else {
                        &right[*column - left_width]
                    };
                    typed::write_typed_value(&mut row_output, cell, &projection_types[index])?;
                }
                row_output.write_all(b"\n")?;
            }
            position += 1;
            Ok(())
        },
    )?;
    row_output.flush()?;
    Ok(())
}

/// Stream the right input when it is too large to retain, hashing only the
/// smaller left input. SELECT columns still follow their original left/right
/// bindings; only the unordered probe traversal changes.
fn probe_right<H, F>(
    right_path: &Path,
    right_names: &[String],
    right_keys: &[usize],
    left_rows: &[Vec<typed::Cell>],
    hash: &H,
    options: ProbeOptions,
    mut emit: F,
) -> Result<usize, ExecutionError>
where
    H: JoinLookup,
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
        if let Some(local) = column
            .checked_sub(left_width)
            .filter(|&local| local < right_names.len())
        {
            retained.push(local);
        }
    }
    retained.sort_unstable();
    retained.dedup();
    if retained.is_empty() {
        retained.push(0);
    }
    let selected = retained
        .iter()
        .map(|&index| right_names[index].clone())
        .collect::<Vec<_>>();
    let key_indices = right_keys
        .iter()
        .map(|index| retained.binary_search(index).expect("join key retained"))
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(right_path, &selected)?;
    let mut matches = 0usize;
    // Each retained left row can match only its first right row.
    let mut seen_left = options.any.then(|| vec![false; left_rows.len()]);
    let mut remaining_left = seen_left.as_ref().map(|_| hash.total_bucket_items());
    let mut probe_key = Vec::with_capacity(right_keys.len());
    'scan: while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            if batch_key_into(&batch, index, &key_indices, &mut probe_key) {
                if let Some(left_indices) = hash.lookup(probe_key.as_slice()) {
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
                                    &batch[retained
                                        .binary_search(&(column - left_width))
                                        .expect("filter retained")],
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
    let projection_types = join_output_types(left_path, right_path, selected)?;
    let left_meta = inspect_query_source(left_path)?;
    let keep = retained_build_columns(left_names, left_keys, selected, null_filter, 0);
    let left_rows = read_projected_build_rows_using_metadata(
        left_path,
        left_names,
        &keep,
        left_meta.rows,
        MAX_STREAM_BUILD_ROWS,
        MAX_STREAM_BUILD_BYTES,
        Some(&left_meta),
    )?;
    let mut hash = JoinHash::with_capacity(
        left_keys.len(),
        initial_hash_capacity(left_rows.len()),
        single_numeric_join_key(&left_rows, left_keys),
    );
    let mut build_key = Vec::with_capacity(left_keys.len());
    for (index, row) in left_rows.iter().enumerate() {
        insert_build_row(&mut hash, row, left_keys, &mut build_key, index, false);
    }
    if hash.is_empty() {
        let header = selected_header(selected);
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
    let header = selected_header(selected);
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
                    typed::write_typed_value(output, value, &projection_types[index])?;
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
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for OrderedJoinRow {}
impl PartialOrd for OrderedJoinRow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedJoinRow {
    fn cmp(&self, other: &Self) -> Ordering {
        for ((left, right), &(descending, nulls_first)) in self
            .keys
            .iter()
            .zip(&other.keys)
            .zip(self.directions.iter())
        {
            let ordering = typed::compare_order_cells(left, right, descending, nulls_first);
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        self.sequence.cmp(&other.sequence)
    }
}

fn ordered_join_retained_bytes(cells: &[typed::Cell]) -> Result<usize, ExecutionError> {
    cells
        .iter()
        .try_fold(std::mem::size_of_val(cells), |total, cell| {
            total
                .checked_add(match cell {
                    typed::Cell::Text(bytes) => bytes.len(),
                    typed::Cell::Uuid(value) => value.len(),
                    _ => 0,
                })
                .ok_or_else(|| invalid("ordered JOIN output size overflow"))
        })
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
        left_names,
        right_names,
        selected,
        left_keys,
        right_keys,
        null_filter,
    } = columns;
    let projection_types = join_output_types(left_path, right_path, selected)?;
    let right_metadata = inspect_query_source(right_path)?;
    let mut keep = retained_build_columns(
        right_names,
        right_keys,
        selected,
        null_filter,
        left_names.len(),
    );
    // An ORDER BY column can be absent from SELECT but is still needed
    // when comparing the streamed joined rows.
    for &(index, _, _) in order {
        if let Some(local) = index
            .checked_sub(left_names.len())
            .filter(|&index| index < right_names.len())
        {
            keep.push(local);
        }
    }
    keep.sort_unstable();
    keep.dedup();
    let right_rows = read_projected_build_rows_using_metadata(
        right_path,
        right_names,
        &keep,
        right_metadata.rows,
        MAX_STREAM_BUILD_ROWS,
        MAX_STREAM_BUILD_BYTES,
        Some(&right_metadata),
    )?;
    // Reuse the cardinality-aware build for ordered top-K joins.
    // Preserve source match order and ANY first-match semantics.
    let hash = build_join_hash(&right_rows, right_keys, query.any, super::query_workers(4))?;
    let mut heap = BinaryHeap::<OrderedJoinRow>::with_capacity(requested.min(1024));
    let directions: Arc<[(bool, Option<bool>)]> = order
        .iter()
        .map(|(_, desc, nulls)| (*desc, *nulls))
        .collect::<Vec<_>>()
        .into();
    let mut retained = 0usize;
    let mut sequence = 0usize;
    let width = left_names.len();
    let projected_left = selected
        .iter()
        .map(|(index, _, _)| *index)
        .chain(order.iter().map(|(index, _, _)| *index))
        .filter(|&index| index < width)
        .collect::<Vec<_>>();
    probe_inner_projected(
        left_path,
        left_names,
        left_keys,
        &right_rows,
        &hash,
        ProbeOptions {
            materialize_rows: true,
            materialize_after: 0,
            // A small ordered result still needs to inspect every qualifying
            // match, without the unordered 131072-output limit.
            stop_after: Some(usize::MAX),
            any: query.any,
            null_filter,
        },
        Some(&projected_left),
        |left, right| {
            let at = |index: usize| -> &typed::Cell {
                if index < width {
                    &left[index]
                } else {
                    &right[index - width]
                }
            };
            let ordinal = sequence;
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| invalid("ordered JOIN row count overflow"))?;
            // Compare borrowed values before cloning any projected rows.
            if heap.len() == requested {
                if let Some(worst) = heap.peek() {
                    let mut comparison = Ordering::Equal;
                    for (position, &(index, descending, nulls_first)) in order.iter().enumerate() {
                        comparison = typed::compare_order_cells(
                            at(index),
                            &worst.keys[position],
                            descending,
                            nulls_first,
                        );
                        if comparison != Ordering::Equal {
                            break;
                        }
                    }
                    // Later equal keys cannot displace an earlier winner.
                    if comparison != Ordering::Less {
                        return Ok(());
                    }
                }
            }
            let keys: Vec<typed::Cell> = order
                .iter()
                .map(|(index, _, _)| at(*index).clone())
                .collect();
            let values: Vec<typed::Cell> = selected
                .iter()
                .map(|(index, _, _)| at(*index).clone())
                .collect();
            let bytes = ordered_join_retained_bytes(&keys)?
                .checked_add(ordered_join_retained_bytes(&values)?)
                .ok_or_else(|| invalid("ordered JOIN output size overflow"))?;
            let candidate = OrderedJoinRow {
                values,
                keys,
                directions: directions.clone(),
                sequence: ordinal,
                retained_bytes: bytes,
            };
            if heap.len() == requested && heap.peek().is_some_and(|worst| candidate >= *worst) {
                return Ok(());
            }
            let removed = if heap.len() == requested {
                heap.peek().map_or(0, |row| row.retained_bytes)
            } else {
                0
            };
            let total = retained
                .checked_sub(removed)
                .and_then(|value| value.checked_add(bytes))
                .ok_or_else(|| invalid("ordered JOIN output size overflow"))?;
            if total > MAX_STREAM_BUILD_BYTES {
                return Err(invalid(
                    "ordered JOIN exceeds its retained-row memory budget",
                ));
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
    let header = selected_header(selected);
    writeln!(output, "{header}")?;
    for row in heap.into_sorted_vec().into_iter().skip(query.offset) {
        for (index, value) in row.values.iter().enumerate() {
            if index != 0 {
                output.write_all(b"\t")?;
            }
            typed::write_typed_value(output, value, &projection_types[index])?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

/// Probe independent immutable fact parts concurrently for INNER JOIN COUNT.
/// Partial counts need no cross-part output ordering or matched-row state.
fn parallel_probe_inner_count(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    hash: &JoinHash,
    any: bool,
    null_filter: Option<(usize, bool)>,
) -> Result<Option<usize>, ExecutionError> {
    let parts = rusthouse_storage::query_parts(left_path)?;
    let workers = super::query_workers(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let counts = std::thread::scope(|scope| -> Result<_, ExecutionError> {
        let handles = (0..workers)
            .map(|_| {
                std::thread::Builder::new()
                    .spawn_scoped(scope, || -> Result<usize, ExecutionError> {
                        let mut subtotal = 0usize;
                        while let Some(part) =
                            parts.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                        {
                            let count = probe_inner(
                                part,
                                left_names,
                                left_keys,
                                right_rows,
                                hash,
                                ProbeOptions {
                                    materialize_rows: false,
                                    materialize_after: 0,
                                    stop_after: Some(usize::MAX),
                                    any,
                                    null_filter,
                                },
                                |_, _| Ok(()),
                            )?;
                            subtotal = subtotal
                                .checked_add(count)
                                .ok_or_else(|| invalid("parallel JOIN COUNT overflow"))?;
                        }
                        Ok(subtotal)
                    })
                    .map_err(|error| {
                        invalid(format!("cannot start parallel JOIN probe worker: {error}"))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| invalid("parallel JOIN probe worker panicked"))?
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    let count = counts.into_iter().try_fold(0usize, |sum, next| {
        sum.checked_add(next)
            .ok_or_else(|| invalid("parallel JOIN COUNT overflow"))
    })?;
    Ok(Some(count))
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
        left_names,
        right_names,
        left_keys,
        right_keys,
        null_filter,
        ..
    } = columns;
    if query.limit == Some(0) || query.offset > 0 {
        writeln!(
            output,
            "{}",
            query.aggregate_alias.as_deref().unwrap_or("count")
        )?;
        return Ok(());
    }
    if left_count == 0 || right_count == 0 {
        writeln!(
            output,
            "{}\n0",
            query.aggregate_alias.as_deref().unwrap_or("count")
        )?;
        return Ok(());
    }
    let count = if right_count <= MAX_STREAM_BUILD_ROWS
        && (right_count <= left_count || left_count > MAX_STREAM_BUILD_ROWS)
    {
        let keep =
            count_build_columns(right_keys, null_filter, left_names.len(), right_names.len());
        let right_rows = read_projected_build_rows(
            right_path,
            right_names,
            &keep,
            right_count,
            MAX_STREAM_BUILD_ROWS,
            MAX_STREAM_BUILD_BYTES,
        )?;
        let hash = build_join_hash(&right_rows, right_keys, query.any, super::query_workers(4))?;
        if let Some(count) = parallel_probe_inner_count(
            left_path,
            left_names,
            left_keys,
            &right_rows,
            &hash,
            query.any,
            null_filter,
        )? {
            count
        } else {
            probe_inner(
                left_path,
                left_names,
                left_keys,
                &right_rows,
                &hash,
                ProbeOptions {
                    materialize_rows: false,
                    materialize_after: 0,
                    stop_after: Some(usize::MAX),
                    any: query.any,
                    null_filter,
                },
                |_, _| Ok(()),
            )?
        }
    } else {
        let keep = count_build_columns(left_keys, null_filter, 0, left_names.len());
        let left_rows = read_projected_build_rows(
            left_path,
            left_names,
            &keep,
            left_count,
            MAX_STREAM_BUILD_ROWS,
            MAX_STREAM_BUILD_BYTES,
        )?;
        // Preserve every left-side duplicate for the right-side COUNT probe.
        let hash = build_join_hash(&left_rows, left_keys, false, super::query_workers(4))?;
        probe_right(
            right_path,
            right_names,
            right_keys,
            &left_rows,
            &hash,
            ProbeOptions {
                materialize_rows: false,
                materialize_after: 0,
                stop_after: Some(usize::MAX),
                any: query.any,
                null_filter,
            },
            |_, _| Ok(()),
        )?
    };
    writeln!(
        output,
        "{}\n{count}",
        query.aggregate_alias.as_deref().unwrap_or("count")
    )?;
    Ok(())
}

/// Prefer the smaller build side for strongly skewed unordered INNER joins.
/// ANY must preserve its existing build ordering and first-match behavior.
fn prefer_left_hash_build(left_rows: u64, right_rows: u64, any: bool) -> bool {
    !any && left_rows > 0
        && left_rows <= MAX_STREAM_BUILD_ROWS
        && left_rows < right_rows
        && (right_rows > MAX_STREAM_BUILD_ROWS || right_rows >= left_rows.saturating_mul(4))
}

fn retain_or_count(
    pairs: &mut Vec<Pair>,
    count: &mut u64,
    count_only: bool,
    pair: Pair,
) -> Result<(), ExecutionError> {
    if count_only {
        *count = count
            .checked_add(1)
            .ok_or_else(|| invalid("JOIN count overflow"))?;
        Ok(())
    } else {
        add_pair(pairs, pair)
    }
}

/// Evaluate a two-table JOIN using typed values, not serialized TSV.

#[cfg(test)]
mod tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn projected_hash_build_retains_using_fallback_on_either_side() {
        let names = vec!["id".to_owned(), "payload".to_owned(), "key".to_owned()];
        let selected = vec![(3, "id".to_owned(), Some(0))];
        assert_eq!(
            retained_build_columns(&names, &[2], &selected, None, 0),
            vec![0, 2]
        );
        assert_eq!(
            retained_build_columns(&names, &[2], &selected, None, 3),
            vec![0, 2]
        );
    }

    #[test]
    fn projected_join_build_initial_capacity_is_bounded() {
        assert_eq!(initial_join_row_capacity(0), 0);
        assert_eq!(initial_join_row_capacity(25), 25);
        assert_eq!(initial_join_row_capacity(262_144), 4_096);
        assert_eq!(initial_join_row_capacity(u64::MAX), 4_096);
    }

    #[test]
    fn streaming_right_preserves_unmatched_rows_and_key_fallbacks() {
        use rusthouse_storage::{Column, write_wide_part};
        use std::fs;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rusthouse-right-stream-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let right = root.join("right");
        fs::create_dir_all(&root).unwrap();
        let schema = [Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }];
        write_wide_part(
            &right,
            &schema,
            &[
                vec![Some("1".into())],
                vec![Some("3".into())],
                vec![Some("2".into())],
            ],
        )
        .unwrap();
        let left_rows = vec![
            vec![typed::Cell::Numeric(Number::Unsigned(1))],
            vec![typed::Cell::Numeric(Number::Unsigned(1))],
        ];
        let mut hash = HashMap::new();
        hash.insert(
            vec![typed::Cell::Numeric(Number::Unsigned(1)).key()],
            Bucket::Many(Box::new(vec![0, 1])),
        );
        let mut seen = Vec::new();
        let count = probe_right_outer(
            &right,
            &["id".into()],
            &[0],
            &left_rows,
            1,
            &hash,
            ProbeOptions {
                materialize_rows: true,
                materialize_after: 0,
                stop_after: None,
                any: false,
                null_filter: None,
            },
            |left, right| {
                let id = match &right[0] {
                    typed::Cell::Numeric(Number::Unsigned(id)) => *id,
                    _ => panic!("expected right id"),
                };
                seen.push((id, matches!(&left[0], typed::Cell::Null)));
                Ok(())
            },
        )
        .unwrap();
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
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let names: Vec<String> = vec!["id".into(), "unused".into(), "label".into()];
        let schema = vec![
            Column {
                name: names[0].clone(),
                data_type: "UInt64".into(),
            },
            Column {
                name: names[1].clone(),
                data_type: "String".into(),
            },
            Column {
                name: names[2].clone(),
                data_type: "String".into(),
            },
        ];
        write_wide_part(
            &root,
            &schema,
            &[vec![
                Some("5".into()),
                Some("unneeded-heavy-payload".repeat(100)),
                Some("ok".into()),
            ]],
        )
        .unwrap();
        let selected = vec![
            (3, "fact.id".into(), None),
            (5, "dimension.label".into(), None),
        ];
        let keep = retained_build_columns(&names, &[0], &selected, None, 3);
        assert_eq!(keep, vec![0, 2]);
        let rows = read_projected_build_rows(&root, &names, &keep, 1, 10, 4_096).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0][1], typed::Cell::Null));
        assert!(matches!(&rows[0][2], typed::Cell::Text(value) if value == b"ok"));
        // The already validated snapshot must return the same projected rows.
        let metadata = inspect_query_source(&root).unwrap();
        let reused = read_projected_build_rows_using_metadata(
            &root,
            &names,
            &keep,
            1,
            10,
            4_096,
            Some(&metadata),
        )
        .unwrap();
        assert_eq!(reused.len(), rows.len());
        assert!(matches!(&reused[0][2], typed::Cell::Text(value) if value == b"ok"));
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
        assert_eq!(
            count_build_columns(&[2, 0, 2], Some((4, true)), 3, 5),
            vec![0, 1, 2]
        );
        assert_eq!(
            count_build_columns(&[0], Some((7, false)), 3, 5),
            vec![0, 4]
        );
        assert_eq!(count_build_columns(&[], None, 0, 4), vec![0]);
        assert!(count_build_columns(&[], None, 0, 0).is_empty());
    }
}

#[cfg(test)]
mod hash_capacity_regressions {
    use super::initial_hash_capacity;

    #[test]
    fn hash_buckets_grow_with_distinct_keys_not_input_row_estimates() {
        assert_eq!(initial_hash_capacity(0), 0);
        assert_eq!(initial_hash_capacity(11), 11);
        assert_eq!(initial_hash_capacity(262_144), 4096);
        assert_eq!(initial_hash_capacity(usize::MAX), 4096);
    }
}

#[cfg(test)]
mod any_join_build_tests {
    use super::*;

    #[test]
    fn any_join_retains_only_first_duplicate_build_index() {
        let mut hash = HashMap::new();
        let key = vec![typed::Key::Text(b"heavy-hitter".to_vec())];
        insert_build_key(
            &mut hash,
            vec![typed::Key::Text(b"heavy-hitter".to_vec())],
            1,
            true,
        );
        insert_build_key(&mut hash, key, 2, true);
        assert_eq!(
            hash.values()
                .next()
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![1]
        );
        insert_build_key(
            &mut hash,
            vec![typed::Key::Text(b"heavy-hitter".to_vec())],
            3,
            false,
        );
        assert_eq!(
            hash.values()
                .next()
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
    }
}

#[cfg(test)]
mod adaptive_join_build_side_tests {
    use super::*;
    #[test]
    fn selects_smaller_build_only_when_safe() {
        assert!(prefer_left_hash_build(10_000, 200_000, false));
        assert!(prefer_left_hash_build(3, 400_000, false));
        assert!(!prefer_left_hash_build(0, 40_000, false));
        assert!(!prefer_left_hash_build(200_000, 10_000, false));
        assert!(!prefer_left_hash_build(40_000, 60_000, false));
        assert!(!prefer_left_hash_build(10_000, 200_000, true));
        assert!(!prefer_left_hash_build(
            MAX_STREAM_BUILD_ROWS + 1,
            u64::MAX,
            false
        ));
    }
}

#[cfg(test)]
mod selected_join_header_tests {
    use super::selected_header;

    #[test]
    fn preserves_empty_single_and_multiple_column_headers() {
        assert_eq!(selected_header(&[]), "");
        assert_eq!(selected_header(&[(0, "one".into(), None)]), "one");
        assert_eq!(
            selected_header(&[(0, "left.id".into(), None), (1, "right.id".into(), Some(0))]),
            "left.id\tright.id"
        );
    }
}

#[cfg(test)]
mod reusable_join_build_key_tests {
    use super::*;

    #[test]
    fn duplicate_text_keys_reuse_scratch_and_preserve_all_join_matches() {
        let mut hash = JoinHash::with_capacity(1, 0, false);
        let mut scratch = Vec::new();
        let rows = [
            vec![typed::Cell::Text(b"repeat-key".to_vec())],
            vec![typed::Cell::Text(b"repeat-key".to_vec())],
            vec![typed::Cell::Null],
            vec![typed::Cell::Text(b"another".to_vec())],
            vec![typed::Cell::Text(b"repeat-key".to_vec())],
        ];
        for (index, row) in rows.iter().enumerate() {
            insert_build_row(&mut hash, row, &[0], &mut scratch, index, false);
        }
        assert_eq!(hash.len(), 2);
        let lookup = vec![typed::Key::Text(b"repeat-key".to_vec())];
        assert_eq!(
            hash.lookup(lookup.as_slice())
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![0, 1, 4],
        );

        let mut any_hash = JoinHash::with_capacity(1, 0, false);
        for (index, row) in rows.iter().enumerate() {
            insert_build_row(&mut any_hash, row, &[0], &mut scratch, index, true);
        }
        assert_eq!(
            any_hash
                .lookup(lookup.as_slice())
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![0],
        );
    }

    #[test]
    fn numeric_keys_preserve_cross_signedness_matches_and_skip_nulls() {
        let rows = [
            vec![typed::Cell::Numeric(Number::Signed(7))],
            vec![typed::Cell::Numeric(Number::Unsigned(7))],
            vec![typed::Cell::Null],
            vec![typed::Cell::Numeric(Number::Signed(-7))],
            vec![typed::Cell::Numeric(Number::Unsigned(9))],
        ];
        let hash = build_join_hash(&rows, &[0], false, 1).unwrap();
        assert_eq!(hash.len(), 3);

        let positive = [typed::Key::Numeric(NumberKey::Unsigned(7))];
        assert_eq!(
            hash.lookup(&positive)
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![0, 1],
        );
        let negative = [typed::Key::Numeric(NumberKey::Signed(-7))];
        assert_eq!(
            hash.lookup(&negative)
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![3]
        );
    }

    #[test]
    fn unsigned_composite_pair_keys_keep_duplicates_without_allocating_vectors() {
        let key = [
            typed::Key::Numeric(NumberKey::Unsigned(3)),
            typed::Key::Numeric(NumberKey::Unsigned(7)),
        ];
        let mut hash = JoinHash::with_unsigned_pair_capacity(0);
        hash.insert_new(&key, 3);

        assert!(matches!(&hash, JoinHash::CompositeUnsignedPair(_)));
        hash.lookup_mut(&key).unwrap().push(8);
        assert_eq!(
            hash.lookup(&key)
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![3, 8]
        );
        let other = [
            typed::Key::Numeric(NumberKey::Unsigned(3)),
            typed::Key::Numeric(NumberKey::Unsigned(9)),
        ];
        assert!(hash.lookup(&other).is_none());
    }

    #[test]
    fn non_numeric_composite_pairs_keep_vector_lookup_and_duplicates() {
        let key = [
            typed::Key::Text(b"part".to_vec()),
            typed::Key::Numeric(NumberKey::Unsigned(7)),
        ];
        let mut hash = JoinHash::with_capacity(2, 0, false);
        hash.insert_new(&key, 3);

        assert!(matches!(&hash, JoinHash::Composite(_)));
        hash.lookup_mut(&key).unwrap().push(8);
        assert_eq!(
            hash.lookup(&key)
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![3, 8]
        );
        let other = [
            typed::Key::Text(b"part".to_vec()),
            typed::Key::Numeric(NumberKey::Unsigned(9)),
        ];
        assert!(hash.lookup(&other).is_none());
    }

    #[test]
    fn unsigned_composite_detection_skips_nulls_and_rejects_other_types() {
        let numeric_rows = [
            vec![typed::Cell::Null, typed::Cell::Null],
            vec![
                typed::Cell::Numeric(Number::Unsigned(1)),
                typed::Cell::Numeric(Number::Signed(2)),
            ],
        ];
        assert!(unsigned_composite_join_key(&numeric_rows, &[0, 1]));

        let text_rows = [vec![
            typed::Cell::Text(b"a".to_vec()),
            typed::Cell::Numeric(Number::Unsigned(2)),
        ]];
        let negative_rows = [vec![
            typed::Cell::Numeric(Number::Signed(-1)),
            typed::Cell::Numeric(Number::Unsigned(2)),
        ]];
        assert!(!unsigned_composite_join_key(&text_rows, &[0, 1]));
        assert!(!unsigned_composite_join_key(&negative_rows, &[0, 1]));
        assert!(!unsigned_composite_join_key(&numeric_rows, &[0]));
    }

    #[test]
    fn wider_composite_keys_keep_the_vector_fallback() {
        let key = [
            typed::Key::Numeric(NumberKey::Unsigned(1)),
            typed::Key::Numeric(NumberKey::Unsigned(2)),
            typed::Key::Numeric(NumberKey::Unsigned(3)),
        ];
        let mut hash = JoinHash::with_capacity(key.len(), 0, false);
        hash.insert_new(&key, 5);
        assert!(matches!(&hash, JoinHash::Composite(_)));
        assert_eq!(
            hash.lookup(&key)
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![5]
        );
    }
}

#[cfg(test)]
mod parallel_join_build_tests {
    use super::*;

    #[test]
    fn parallel_hash_build_preserves_all_and_any_row_order() {
        let rows = (0..16_384)
            .map(|index| vec![typed::Cell::Numeric(Number::Unsigned((index % 5) as u128))])
            .collect::<Vec<_>>();
        let serial = build_join_hash(&rows, &[0], false, 1).unwrap();
        let parallel = build_join_hash(&rows, &[0], false, 4).unwrap();
        assert_eq!(serial.len(), parallel.len());
        for value in 0..5 {
            let key = [typed::Key::Numeric(NumberKey::Unsigned(value))];
            let expected = serial.lookup(&key).unwrap();
            let actual = parallel.lookup(&key).unwrap();
            assert_eq!(
                actual.iter().copied().collect::<Vec<_>>(),
                expected.iter().copied().collect::<Vec<_>>()
            );
        }
        let any = build_join_hash(&rows, &[0], true, 4).unwrap();
        assert_eq!(any.len(), 5);
        for value in 0..5 {
            let key = [typed::Key::Numeric(NumberKey::Unsigned(value))];
            let bucket = any.lookup(&key).unwrap();
            assert_eq!(
                bucket.iter().next(),
                serial.lookup(&key).unwrap().iter().next()
            );
            assert_eq!(bucket.len(), 1);
        }
    }
}

#[cfg(test)]
mod selective_join_probe_tests {
    use super::*;

    #[test]
    fn source_slots_stay_stable_when_unreferenced_columns_are_skipped() {
        let batch = vec![
            TypedColumn::Numeric(rusthouse_storage::NumericColumn::UInt64(vec![4])),
            TypedColumn::Numeric(rusthouse_storage::NumericColumn::UInt64(vec![9])),
        ];
        let cells = reconstruct_join_projection(&batch, 0, &[0, 2], 3, &[false, false, true]);
        assert!(matches!(cells[0], typed::Cell::Null));
        assert!(matches!(cells[1], typed::Cell::Null));
        assert!(matches!(
            cells[2],
            typed::Cell::Numeric(Number::Unsigned(9))
        ));
    }
}

#[cfg(test)]
mod grouped_count_column_guard_tests {
    use super::*;

    #[test]
    fn rejects_manually_modified_outer_join_plans_before_reading() {
        let mut query = crate::parse_join(
            "SELECT a.k, COUNT(b.v) AS n FROM a LEFT JOIN b ON a.k = b.k GROUP BY a.k",
        )
        .unwrap()
        .unwrap();
        query.kind = JoinType::Full;
        let mut output = Vec::new();
        let error = execute_join(
            &query,
            Path::new("missing"),
            Path::new("missing"),
            &mut output,
        )
        .unwrap_err();
        assert!(error.to_string().contains("grouped COUNT(column)"));
        assert!(output.is_empty());
    }
}
