mod aggregate_plans;
mod external_grouped;
mod bitmap_distinct;
mod hyperloglog;

#[cfg(test)]
mod dynamic_part_tests {
    use super::*;

    #[test]
    fn workers_claim_every_part_once_even_for_uneven_shares() {
        let cursor = AtomicUsize::new(0);
        let mut indices = std::thread::scope(|scope| {
            let handles = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        let mut claimed = Vec::new();
                        while let Some(index) = next_part_index(&cursor, 257) {
                            claimed.push(index);
                        }
                        claimed
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        indices.sort_unstable();
        assert_eq!(indices, (0..257).collect::<Vec<_>>());
        assert_eq!(next_part_index(&cursor, 257), None);
    }
}

use self::aggregate_plans::*;
use external_grouped::ExternalGroupCount64;
use bitmap_distinct::BitmapDistinct32;

use crate::{AggregateFunction, Projection, Select, Source};
use rusthouse_storage::{NumericColumn, NumericQueryReader, PartError, inspect_query_source};
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

mod grouped_distinct;
mod grouped_approx_distinct;
mod grouped_multi_approx_distinct;
mod grouped_generated;
mod grouped_multi;
mod grouped_multi_distinct;
mod grouped_numeric_keys;
mod grouped_numeric_multi;
mod join;
mod multi_join;
mod predicates;
mod sort;
mod typed;
pub use join::execute_join;
pub use multi_join::execute_multi_join;
use sort::{MAX_SORTED_ROWS, SortedRows};
mod multi_aggregate;
mod string_length;
mod typed_aggregate;
#[cfg(test)]
use predicates::{CompiledTest, compile_in_test};
use predicates::{
    batch_matches, compile_numbers_predicates, compile_part_where, constant_predicate_result,
    count_batch_matches, count_monotonic_numbers, excluded_generated_positions,
    monotonic_range_indices, number_matches, number_matches_all, parse_numeric,
    sparse_generated_positions,
};

#[derive(Debug)]
pub enum ExecutionError {
    Storage(PartError),
    Io(io::Error),
    Invalid(String),
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Invalid(message) => write!(f, "SQL execution error: {message}"),
        }
    }
}

impl Error for ExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<PartError> for ExecutionError {
    fn from(error: PartError) -> Self {
        Self::Storage(error)
    }
}

impl From<io::Error> for ExecutionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Copy)]
enum Number {
    Unsigned(u128),
    Signed(i128),
    Float(f64),
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsigned(value) => write!(f, "{value}"),
            Self::Signed(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
        }
    }
}

#[derive(PartialEq, Eq, Hash)]
enum NumberKey {
    Unsigned(u128),
    Signed(i128),
    Float(u64),
}

impl From<Number> for NumberKey {
    fn from(value: Number) -> Self {
        match value {
            Number::Unsigned(value) => Self::Unsigned(value),
            Number::Signed(value) => Self::Signed(value),
            Number::Float(0.0) => Self::Float(0),
            Number::Float(value) if value.is_nan() => Self::Float(f64::NAN.to_bits()),
            Number::Float(value) => Self::Float(value.to_bits()),
        }
    }
}

const MAX_DISTINCT_ROWS: usize = 200_000;
const MAX_MEDIAN_ROWS: usize = 100_000;

fn remember_distinct(
    seen: &mut HashSet<Vec<NumberKey>>,
    values: &[Number],
) -> Result<bool, ExecutionError> {
    let key = values.iter().copied().map(NumberKey::from).collect();
    // At capacity, duplicates are still valid but new unique rows are not.
    if seen.len() >= MAX_DISTINCT_ROWS {
        if seen.contains(&key) {
            return Ok(false);
        }
        return Err(ExecutionError::Invalid(
            "DISTINCT supports at most 200000 unique rows".into(),
        ));
    }
    Ok(seen.insert(key))
}

fn write_header<W: Write>(
    query: &Select,
    output: &mut W,
    default: &str,
) -> Result<(), ExecutionError> {
    if let Some((source, target)) = &query.rename {
        let columns: Vec<&str> = default.split('\t').collect();
        if !columns.contains(&source.as_str()) {
            return Err(ExecutionError::Invalid(format!(
                "unknown RENAME column: {source}"
            )));
        }
        if source != target && columns.contains(&target.as_str()) {
            return Err(ExecutionError::Invalid(format!(
                "RENAME target already exists: {target}"
            )));
        }
        let header = columns
            .into_iter()
            .map(|column| {
                if column == source.as_str() {
                    target.as_str()
                } else {
                    column
                }
            })
            .collect::<Vec<_>>()
            .join("\t");
        writeln!(output, "{header}")?;
    } else {
        writeln!(output, "{}", query.alias.as_deref().unwrap_or(default))?;
    }
    Ok(())
}

/// Reuse the typed SELECT predicate semantics for bounded local mutations.
pub struct RowPredicate {
    branches: Vec<Vec<typed::Condition>>,
}

impl RowPredicate {
    pub fn new(
        query: &Select,
        columns: &[rusthouse_storage::Column],
    ) -> Result<Self, ExecutionError> {
        let indices = columns
            .iter()
            .enumerate()
            .map(|(index, column)| (column.name.clone(), index))
            .collect::<HashMap<_, _>>();
        let kinds = columns
            .iter()
            .map(|column| (column.name.clone(), column.data_type.clone()))
            .collect::<HashMap<_, _>>();
        let branches = std::iter::once(query.predicates.as_slice())
            .chain(query.or_predicates.iter().map(Vec::as_slice))
            .map(|predicates| typed::compile(predicates, &indices, &kinds))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { branches })
    }

    pub fn matches(&self, batch: &[rusthouse_storage::TypedColumn], row: usize) -> bool {
        typed::passes(batch, row, &self.branches)
    }
}

pub fn execute<W: Write>(
    query: &Select,
    part: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let mut buffered = BufWriter::new(output);
    execute_into(query, part, &mut buffered)?;
    buffered.flush()?;
    Ok(())
}

/// Atomically hand out independent part indices to workers. This avoids
/// stranding a worker on one large immutable part while other workers finish.
fn next_part_index(cursor: &AtomicUsize, part_count: usize) -> Option<usize> {
    let index = cursor.fetch_add(1, AtomicOrdering::Relaxed);
    (index < part_count).then_some(index)
}

/// Evaluate filtered COUNT independently on immutable parts, then reduce the
/// partial counts. Skip small snapshots where thread startup costs dominate.
/// An unfiltered COUNT already has a cheaper metadata-only path.
/// Compile numeric WHERE predicates once and scan immutable parts directly,
/// without restarting the SQL executor or parsing tabular output per part.
/// Text and nullable filters retain the existing general-purpose fallback.
fn parallel_numeric_filtered_count(
    query: &Select,
    parts: &[std::path::PathBuf],
    metadata: &rusthouse_storage::PartMetadata,
    workers: usize,
) -> Result<Option<u64>, ExecutionError> {
    let supported = |name: &str| -> Result<bool, ExecutionError> {
        let column = metadata
            .columns
            .iter()
            .find(|column| column.name == name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
        Ok(matches!(
            column.data_type.as_str(),
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
                | "Float32"
                | "Float64"
        ))
    };
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !supported(predicate.column())? {
            return Ok(None);
        }
    }
    let mut columns = Vec::new();
    let branches = compile_part_where(query, &mut columns, |name, literal| {
        let column = metadata
            .columns
            .iter()
            .find(|column| column.name == name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
        parse_numeric(&column.data_type, literal)
    })?;
    if columns.is_empty() {
        return Ok(None);
    }
    let next = AtomicUsize::new(0);
    let count = std::thread::scope(|scope| -> Result<u64, ExecutionError> {
        let handles = (0..workers)
            .map(|_| {
                let columns = &columns;
                let branches = &branches;
                let next = &next;
                scope.spawn(move || -> Result<u64, ExecutionError> {
                    let mut partial = 0u64;
                    while let Some(index) = next_part_index(next, parts.len()) {
                        let mut reader =
                            rusthouse_storage::NumericPartReader::open(&parts[index], columns)?;
                        while let Some(batch) = reader.next_batch()? {
                            partial = partial
                                .checked_add(count_batch_matches(&batch, branches) as u64)
                                .ok_or_else(|| {
                                    ExecutionError::Invalid("parallel COUNT overflow".into())
                                })?;
                        }
                    }
                    Ok(partial)
                })
            })
            .collect::<Vec<_>>();
        let mut total = 0u64;
        for handle in handles {
            let partial = handle
                .join()
                .map_err(|_| ExecutionError::Invalid("parallel COUNT worker panicked".into()))??;
            total = total
                .checked_add(partial)
                .ok_or_else(|| ExecutionError::Invalid("parallel COUNT overflow".into()))?;
        }
        Ok(total)
    })?;
    Ok(Some(count))
}

/// A large single immutable part can use all CPU workers too. Divide its
/// validated adaptive granules into disjoint ranges; each worker owns reader
/// state and decompresses only its assigned marks.
fn parallel_single_part_filtered_count(
    query: &Select,
    part: &Path,
    metadata: &rusthouse_storage::PartMetadata,
    workers: usize,
) -> Result<Option<u64>, ExecutionError> {
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let selected = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == predicate.column())
            .ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown WHERE column: {}", predicate.column()))
            })?;
        if !matches!(
            selected.data_type.as_str(),
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
                | "Float32"
                | "Float64"
        ) {
            return Ok(None);
        }
    }
    let mut columns = Vec::new();
    let branches = compile_part_where(query, &mut columns, |name, text| {
        let selected = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
        parse_numeric(&selected.data_type, text)
    })?;
    if columns.is_empty() {
        return Ok(None);
    }

    let reference = rusthouse_storage::NumericPartReader::open(part, &columns)?;
    let marks = reference.mark_count();
    // A few marks are faster on the serial path. Avoid extra decompression
    // readers unless there is enough work to amortize startup.
    const MIN_PARALLEL_MARKS: usize = 64;
    if marks < MIN_PARALLEL_MARKS || workers < 2 {
        return Ok(None);
    }
    let chunk = marks.div_ceil(workers);
    let count = std::thread::scope(|scope| -> Result<u64, ExecutionError> {
        let handles = (0..marks)
            .step_by(chunk)
            .map(|begin| {
                let end = (begin + chunk).min(marks);
                let columns = &columns;
                let branches = &branches;
                scope.spawn(move || -> Result<u64, ExecutionError> {
                    let mut reader = rusthouse_storage::NumericPartReader::open(part, columns)?;
                    reader.seek_to_mark(begin)?;
                    let mut subtotal = 0u64;
                    for _ in begin..end {
                        if let Some(batch) = reader.next_batch()? {
                            subtotal = subtotal
                                .checked_add(count_batch_matches(&batch, branches) as u64)
                                .ok_or_else(|| {
                                    ExecutionError::Invalid("parallel COUNT overflow".into())
                                })?;
                        }
                    }
                    Ok(subtotal)
                })
            })
            .collect::<Vec<_>>();
        let mut total = 0u64;
        for handle in handles {
            let subtotal = handle
                .join()
                .map_err(|_| ExecutionError::Invalid("parallel COUNT worker panicked".into()))??;
            total = total
                .checked_add(subtotal)
                .ok_or_else(|| ExecutionError::Invalid("parallel COUNT overflow".into()))?;
        }
        Ok(total)
    })?;
    Ok(Some(count))
}

fn parallel_filtered_count(query: &Select, source: &Path) -> Result<Option<u64>, ExecutionError> {
    if !matches!(query.projection, Projection::Count)
        || (query.predicates.is_empty() && query.or_predicates.is_empty())
        || query.limit.is_some()
        || query.offset.is_some()
        || query.distinct
        || query.order_by.is_some()
        || !query.order_by_extra.is_empty()
        || query.having.is_some()
    {
        return Ok(None);
    }
    let parts = rusthouse_storage::query_parts(source)?;
    const MIN_PARALLEL_PARTS: usize = 32;
    const MAX_WORKERS: usize = 4;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(MAX_WORKERS);
    if workers < 2 {
        return Ok(None);
    }
    if parts.len() == 1 {
        let metadata = inspect_query_source(source)?;
        if let Some(count) =
            parallel_single_part_filtered_count(query, &parts[0], &metadata, workers)?
        {
            return Ok(Some(count));
        }
    }
    if parts.len() < MIN_PARALLEL_PARTS {
        return Ok(None);
    }
    // Validate all part schemas before executing any partial count. A mismatched
    // or incomplete snapshot must never produce a plausible partial answer.
    let metadata = inspect_query_source(source)?;
    if let Some(count) = parallel_numeric_filtered_count(query, &parts, &metadata, workers)? {
        return Ok(Some(count));
    }
    // Text and nullable filters use the general executor. Balance uneven
    // part sizes by claiming one verified part at a time.
    let next = AtomicUsize::new(0);
    let total = std::thread::scope(|scope| -> Result<u64, ExecutionError> {
        let handles = (0..workers)
            .map(|_| {
                let next = &next;
                let parts = &parts;
                scope.spawn(move || -> Result<u64, ExecutionError> {
                    let mut sum = 0u64;
                    let mut buffer = Vec::new();
                    while let Some(index) = next_part_index(next, parts.len()) {
                        buffer.clear();
                        // A single-part source avoids re-entering this parallel path.
                        execute(query, Some(&parts[index]), &mut buffer)?;
                        let header_end =
                            buffer
                                .iter()
                                .position(|&byte| byte == b'\n')
                                .ok_or_else(|| {
                                    ExecutionError::Invalid("malformed part COUNT header".into())
                                })?;
                        let value = buffer
                            .get(header_end + 1..)
                            .and_then(|tail| tail.strip_suffix(b"\n"))
                            .filter(|digits| {
                                !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
                            })
                            .ok_or_else(|| {
                                ExecutionError::Invalid("malformed part COUNT value".into())
                            })?;
                        let count = std::str::from_utf8(value)
                            .map_err(|_| {
                                ExecutionError::Invalid("invalid part COUNT encoding".into())
                            })?
                            .parse::<u64>()
                            .map_err(|_| ExecutionError::Invalid("part COUNT overflow".into()))?;
                        sum = sum.checked_add(count).ok_or_else(|| {
                            ExecutionError::Invalid("parallel COUNT overflow".into())
                        })?;
                    }
                    Ok(sum)
                })
            })
            .collect::<Vec<_>>();
        let mut total = 0u64;
        for handle in handles {
            let partial = handle
                .join()
                .map_err(|_| ExecutionError::Invalid("parallel COUNT worker panicked".into()))??;
            total = total
                .checked_add(partial)
                .ok_or_else(|| ExecutionError::Invalid("parallel COUNT overflow".into()))?;
        }
        Ok(total)
    })?;
    Ok(Some(total))
}

/// Merge partial integer SUM values with the same overflow behavior as
/// AggregateState. NULL partials represent shards with no matching rows.
fn add_integer_partial(
    total: Option<Number>,
    next: Option<Number>,
) -> Result<Option<Number>, ExecutionError> {
    match (total, next) {
        (None, value) | (value, None) => Ok(value),
        (Some(Number::Signed(a)), Some(Number::Signed(b))) => {
            Ok(Some(Number::Signed(a.checked_add(b).ok_or_else(|| {
                ExecutionError::Invalid("parallel signed SUM overflow".into())
            })?)))
        }
        (Some(Number::Unsigned(a)), Some(Number::Unsigned(b))) => Ok(Some(Number::Unsigned(
            a.checked_add(b)
                .ok_or_else(|| ExecutionError::Invalid("parallel unsigned SUM overflow".into()))?,
        ))),
        _ => Err(ExecutionError::Invalid("parallel SUM type mismatch".into())),
    }
}

/// Evaluate MIN and MAX over many immutable parts in parallel. The scan
/// applies the same typed numeric predicates as a serial aggregate, while
/// reducing only one extremum from each worker.
fn parallel_integer_minmax(
    query: &Select,
    source: &Path,
) -> Result<Option<Option<Number>>, ExecutionError> {
    let Projection::Aggregate { function, column } = &query.projection else {
        return Ok(None);
    };
    if !matches!(function, AggregateFunction::Min | AggregateFunction::Max)
        || query.distinct
        || query.limit.is_some()
        || query.offset.is_some()
        || query.order_by.is_some()
        || !query.order_by_extra.is_empty()
        || query.having.is_some()
    {
        return Ok(None);
    }
    let parts = rusthouse_storage::query_parts(source)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(4);
    if parts.len() < 32 || workers < 2 {
        return Ok(None);
    }
    let metadata = inspect_query_source(source)?;
    let Some(kind) = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .map(|candidate| candidate.data_type.as_str())
    else {
        return Err(ExecutionError::Invalid(format!(
            "unknown aggregate column: {column}"
        )));
    };
    if !matches!(
        kind,
        "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
    ) {
        return Ok(None);
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let selected = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == predicate.column())
            .ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown WHERE column: {}", predicate.column()))
            })?;
        if !matches!(
            selected.data_type.as_str(),
            "Int8"
                | "Int16"
                | "Int32"
                | "Int64"
                | "Int128"
                | "UInt8"
                | "UInt16"
                | "UInt32"
                | "UInt64"
                | "UInt128"
                | "Float32"
                | "Float64"
        ) {
            return Ok(None);
        }
    }
    let mut names = vec![column.clone()];
    let branches = compile_part_where(query, &mut names, |name, text| {
        let kind = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
        parse_numeric(&kind.data_type, text)
    })?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let next = AtomicUsize::new(0);
    let result = std::thread::scope(|scope| -> Result<Option<Number>, ExecutionError> {
        let handles = (0..workers)
            .map(|_| {
                let next = &next;
                let names = &names;
                let branches = &branches;
                let parts = &parts;
                scope.spawn(move || -> Result<Option<Number>, ExecutionError> {
                    let mut state = AggregateState::new(*function);
                    while let Some(index) = next_part_index(next, parts.len()) {
                        let mut reader =
                            rusthouse_storage::NumericPartReader::open(&parts[index], names)?;
                        while let Some(batch) = reader.next_batch()? {
                            for row in 0..batch[0].len() {
                                if no_filters || batch_matches(&batch, row, branches) {
                                    state.update(number_at(&batch[0], row))?;
                                }
                            }
                        }
                    }
                    Ok(state.value)
                })
            })
            .collect::<Vec<_>>();
        let mut extremum = None;
        for handle in handles {
            if let Some(value) = handle.join().map_err(|_| {
                ExecutionError::Invalid("parallel extremum worker panicked".into())
            })?? {
                let replace = extremum.is_none_or(|previous| {
                    let ordering = compare_numbers(value, previous);
                    if *function == AggregateFunction::Min {
                        ordering.is_lt()
                    } else {
                        ordering.is_gt()
                    }
                });
                if replace {
                    extremum = Some(value);
                }
            }
        }
        Ok(extremum)
    })?;
    Ok(Some(result))
}

/// Process independent frozen parts concurrently for exact integer SUM.
/// Unsupported types and small inputs stay on the existing executor path.
fn parallel_integer_sum(
    query: &Select,
    source: &Path,
) -> Result<Option<Option<Number>>, ExecutionError> {
    let Projection::Aggregate {
        function: AggregateFunction::Sum,
        column,
    } = &query.projection
    else {
        return Ok(None);
    };
    if query.distinct
        || query.limit.is_some()
        || query.offset.is_some()
        || query.order_by.is_some()
        || !query.order_by_extra.is_empty()
        || query.having.is_some()
    {
        return Ok(None);
    }
    let parts = rusthouse_storage::query_parts(source)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(4);
    if workers < 2 || (parts.len() < 32 && parts.len() != 1) {
        return Ok(None);
    }
    let metadata = inspect_query_source(source)?;
    let kind = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .map(|candidate| candidate.data_type.as_str());
    if !matches!(
        kind,
        Some(
            "Int8"
                | "Int16"
                | "Int32"
                | "Int64"
                | "Int128"
                | "UInt8"
                | "UInt16"
                | "UInt32"
                | "UInt64"
                | "UInt128"
        )
    ) {
        return Ok(None);
    }
    // Only native numeric predicates can be handled by the direct part reader.
    // Other filters retain the existing typed executor.
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let Some(selected) = metadata
            .columns
            .iter()
            .find(|item| item.name == predicate.column())
        else {
            return Err(ExecutionError::Invalid(format!(
                "unknown WHERE column: {}",
                predicate.column()
            )));
        };
        if !matches!(
            selected.data_type.as_str(),
            "Int8"
                | "Int16"
                | "Int32"
                | "Int64"
                | "Int128"
                | "UInt8"
                | "UInt16"
                | "UInt32"
                | "UInt64"
                | "UInt128"
                | "Float32"
                | "Float64"
        ) {
            return Ok(None);
        }
    }
    let mut scan_columns = vec![column.clone()];
    let comparisons = compile_part_where(query, &mut scan_columns, |name, literal| {
        let selected = metadata
            .columns
            .iter()
            .find(|item| item.name == name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
        parse_numeric(&selected.data_type, literal)
    })?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    // Unequal part sizes should not leave idle workers waiting for a single
    // large chunk. Claim one part at a time, as in filtered COUNT.
    if parts.len() == 1 {
        let single_part = &parts[0];
        let reference = rusthouse_storage::NumericPartReader::open(single_part, &scan_columns)?;
        let marks = reference.mark_count();
        const MIN_PARALLEL_MARKS: usize = 64;
        if marks < MIN_PARALLEL_MARKS {
            return Ok(None);
        }
        let chunk = marks.div_ceil(workers);
        let sum = std::thread::scope(|scope| -> Result<Option<Number>, ExecutionError> {
            let handles = (0..marks)
                .step_by(chunk)
                .map(|begin| {
                    let end = (begin + chunk).min(marks);
                    let scan_columns = &scan_columns;
                    let comparisons = &comparisons;
                    scope.spawn(move || -> Result<Option<Number>, ExecutionError> {
                        let mut reader =
                            rusthouse_storage::NumericPartReader::open(single_part, scan_columns)?;
                        reader.seek_to_mark(begin)?;
                        let mut state = AggregateState::new(AggregateFunction::Sum);
                        for _ in begin..end {
                            if let Some(batch) = reader.next_batch()? {
                                for row in 0..batch[0].len() {
                                    if no_filters || batch_matches(&batch, row, comparisons) {
                                        state.update(number_at(&batch[0], row))?;
                                    }
                                }
                            }
                        }
                        Ok(state.value)
                    })
                })
                .collect::<Vec<_>>();
            let mut total = None;
            for handle in handles {
                let partial = handle.join().map_err(|_| {
                    ExecutionError::Invalid("parallel SUM worker panicked".into())
                })??;
                total = add_integer_partial(total, partial)?;
            }
            Ok(total)
        })?;
        return Ok(Some(sum));
    }
    let next = AtomicUsize::new(0);
    let sum = std::thread::scope(|scope| -> Result<Option<Number>, ExecutionError> {
        let handles = (0..workers)
            .map(|_| {
                let scan_columns = &scan_columns;
                let comparisons = &comparisons;
                let next = &next;
                let parts = &parts;
                scope.spawn(move || -> Result<Option<Number>, ExecutionError> {
                    let mut result = None;
                    while let Some(index) = next_part_index(next, parts.len()) {
                        // Scan compressed numeric columns directly, without
                        // recursive SQL execution or TSV serialization.
                        let mut reader = rusthouse_storage::NumericPartReader::open(
                            &parts[index],
                            scan_columns,
                        )?;
                        let mut state = AggregateState::new(AggregateFunction::Sum);
                        while let Some(batch) = reader.next_batch()? {
                            for row in 0..batch[0].len() {
                                if no_filters || batch_matches(&batch, row, comparisons) {
                                    state.update(number_at(&batch[0], row))?;
                                }
                            }
                        }
                        result = add_integer_partial(result, state.value)?;
                    }
                    Ok(result)
                })
            })
            .collect::<Vec<_>>();
        let mut total = None;
        for handle in handles {
            let partial = handle
                .join()
                .map_err(|_| ExecutionError::Invalid("parallel SUM worker panicked".into()))??;
            total = add_integer_partial(total, partial)?;
        }
        Ok(total)
    })?;
    Ok(Some(sum))
}

fn execute_into<W: Write>(
    query: &Select,
    part: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if let Projection::Constants(columns) = &query.projection {
        let headers: Vec<_> = columns.iter().map(crate::Constant::header).collect();
        write_header(query, output, &headers.join("\t"))?;
        if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            for (index, column) in columns.iter().enumerate() {
                if index != 0 {
                    output.write_all(b"\t")?;
                }
                let value = match &column.value {
                    crate::ConstantValue::Integer(value) => {
                        typed::Cell::Numeric(Number::Signed(i128::from(*value)))
                    }
                    crate::ConstantValue::Text(value) => {
                        typed::Cell::Text(value.as_bytes().to_vec())
                    }
                    crate::ConstantValue::Null => typed::Cell::Null,
                };
                typed::write_value(output, &value)?;
            }
            writeln!(output)?;
        }
        return Ok(());
    }
    if let Projection::Literal(value) = query.projection {
        write_header(query, output, &value.to_string())?;
        if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            writeln!(output, "{value}")?;
        }
        return Ok(());
    }

    match query.source.as_ref() {
        Some(Source::Numbers(rows)) => return execute_numbers(query, 0, *rows, 1, part, output),
        Some(Source::NumbersRange { start, count }) => {
            return execute_numbers(query, *start, *count, 1, part, output);
        }
        Some(Source::NumbersStepped {
            start,
            length,
            step,
        }) => {
            let rows = length / step + u64::from(length % step != 0);
            return execute_numbers(query, *start, rows, *step, part, output);
        }
        Some(Source::GenerateSeries { start, count, step }) => {
            return execute_numbers(query, *start, *count, *step, part, output);
        }
        Some(Source::Table(table)) if table == "part" => {}
        _ => {
            return Err(ExecutionError::Invalid(
                "only FROM part and FROM numbers(N) are supported".into(),
            ));
        }
    }
    let part = part.ok_or_else(|| {
        ExecutionError::Invalid("pass --part <directory> for table queries".into())
    })?;
    // A single typed scan serves numeric and String columns with fixed 16 KiB state.
    if matches!(query.projection, Projection::ApproxDistinct { .. }) {
        return typed::execute_typed(query, part, output);
    }
    if let Some(count) = parallel_filtered_count(query, part)? {
        write_header(query, output, "count")?;
        writeln!(output, "{count}")?;
        return Ok(());
    }
    if let Some(extremum) = parallel_integer_minmax(query, part)? {
        let Projection::Aggregate { function, column } = &query.projection else {
            unreachable!("parallel_integer_minmax accepts only aggregate projections");
        };
        write_header(query, output, &format!("{}({column})", function.name()))?;
        if let Some(value) = extremum {
            writeln!(output, "{value}")?;
        } else {
            writeln!(output, "NULL")?;
        }
        return Ok(());
    }
    if let Some(sum) = parallel_integer_sum(query, part)? {
        let Projection::Aggregate { column, .. } = &query.projection else {
            unreachable!("parallel_integer_sum only accepts SUM projections");
        };
        write_header(query, output, &format!("sum({column})"))?;
        if let Some(value) = sum {
            writeln!(output, "{value}")?;
        } else {
            writeln!(output, "NULL")?;
        }
        return Ok(());
    }
    if let Projection::GroupedMultiCount { columns } = &query.projection {
        return grouped_multi::execute(query, part, columns, output);
    }
    if matches!(query.projection, Projection::GroupedMultiApproxDistinct { .. }) {
        return grouped_multi_approx_distinct::execute(query, part, output);
    }
    if matches!(query.projection, Projection::GroupedApproxDistinct { .. }) {
        return grouped_approx_distinct::execute(query, part, output);
    }
    if let Projection::GroupedDistinctCount {
        column,
        value_column,
    } = &query.projection
    {
        return grouped_distinct::execute(query, part, column, value_column, output);
    }
    if let Projection::GroupedMultiDistinctCount { columns, value_column } = &query.projection {
        return grouped_multi_distinct::execute(query, part, columns, value_column, output);
    }
    if let Projection::StringLength { column } = &query.projection {
        return string_length::execute(query, part, column, output);
    }
    if let Projection::MultipleAggregates(items) = &query.projection {
        return multi_aggregate::execute(query, part, items, output);
    }
    if matches!(query.projection, Projection::GroupedAggregate { .. }) {
        return typed_aggregate::execute_grouped_aggregate(query, part, output);
    }
    if let Projection::GroupedAggregates { column, items, aliases, filters } = &query.projection {
        return grouped_numeric_multi::execute(query, part, column, items, aliases, filters, output);
    }
    if let Projection::GroupedMultiAggregates { columns, items, aliases, filters } = &query.projection {
        return grouped_numeric_keys::execute(query, part, columns, items, aliases, filters, output);
    }
    if typed::requires_typed_scan(query, part)? {
        if matches!(query.projection, Projection::Aggregate { .. }) {
            return typed_aggregate::execute_typed_aggregate(query, part, output);
        }
        return typed::execute_typed(query, part, output);
    }
    if let Projection::CountColumn { column } = &query.projection {
        return execute_count_column_part(query, part, column, output);
    }
    if let Projection::CountDistinct { column } = &query.projection {
        return execute_count_distinct_part(query, part, column, output);
    }
    if let Projection::Aggregate { function, column } = &query.projection {
        return execute_aggregate_part(query, part, *function, column, output);
    }
    if let Projection::GroupedCount { column } = &query.projection {
        return execute_grouped_part(query, part, column, output);
    }
    let metadata = inspect_query_source(part)?;

    if matches!(query.projection, Projection::Count)
        && query.predicates.is_empty()
        && query.or_predicates.is_empty()
    {
        write_header(query, output, "count")?;
        if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            writeln!(output, "{}", metadata.rows)?;
        }
        return Ok(());
    }

    let projected = match &query.projection {
        Projection::All => metadata
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>(),
        Projection::Columns(names) => names.clone(),
        Projection::Count => Vec::new(),
        Projection::Aggregate { .. }
        | Projection::CountColumn { .. }
        | Projection::CountDistinct { .. }
        | Projection::ApproxDistinct { .. }
        | Projection::GroupedApproxDistinct { .. }
        | Projection::GroupedMultiApproxDistinct { .. }
        | Projection::GroupedCount { .. }
        | Projection::GroupedDistinctCount { .. }
        | Projection::GroupedMultiDistinctCount { .. }
        | Projection::GroupedMultiCount { .. }
        | Projection::GroupedAggregate { .. }
        | Projection::GroupedAggregates { .. }
        | Projection::GroupedMultiAggregates { .. }
        | Projection::MultipleAggregates(_)
        | Projection::StringLength { .. }
        | Projection::Literal(_)
        | Projection::Constants(_) => unreachable!(),
    };
    if query.distinct
        && query.order_terms().any(|order| {
            !projected
                .iter()
                .any(|column| column == query.order_source_column(&order.column))
        })
    {
        return Err(ExecutionError::Invalid(
            "DISTINCT ORDER BY column must appear in SELECT".into(),
        ));
    }
    let mut scan_columns = Vec::new();
    let projected_indices = projected
        .iter()
        .map(|name| {
            if let Some(index) = scan_columns.iter().position(|column| column == name) {
                index
            } else {
                let index = scan_columns.len();
                scan_columns.push(name.clone());
                index
            }
        })
        .collect::<Vec<_>>();
    let comparisons = compile_part_where(query, &mut scan_columns, |column_name, literal| {
        let column = metadata
            .columns
            .iter()
            .find(|column| column.name == column_name)
            .ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown WHERE column: {column_name}"))
            })?;
        parse_numeric(&column.data_type, literal)
    })?;
    // A constant predicate on non-nullable numeric columns can be resolved
    // from validated part row counts without decoding any compressed blocks.
    // Check every referenced column first to retain unknown-column errors.
    if matches!(query.projection, Projection::Count)
        && !query.distinct
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
        && query.having.is_none()
        && scan_columns
            .iter()
            .all(|name| metadata.columns.iter().any(|column| &column.name == name))
    {
        if let Some(matches_all) = constant_predicate_result(&comparisons) {
            write_header(query, output, "count")?;
            if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{}", if matches_all { metadata.rows } else { 0 })?;
            }
            return Ok(());
        }
    }
    let sort_index = query.order_by.as_ref().map(|order| {
        let column_name = query.order_source_column(&order.column);
        if let Some(index) = scan_columns.iter().position(|column| column == column_name) {
            index
        } else {
            scan_columns.push(column_name.to_owned());
            scan_columns.len() - 1
        }
    });
    let mut extra_sort_indices = Vec::new();
    for order in &query.order_by_extra {
        let column_name = query.order_source_column(&order.column);
        let index = if let Some(index) = scan_columns.iter().position(|c| c == column_name) {
            index
        } else {
            scan_columns.push(column_name.to_owned());
            scan_columns.len() - 1
        };
        extra_sort_indices.push(index);
    }
    let mut reader = NumericQueryReader::open_with_metadata(part, &scan_columns, &metadata)?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let mut distinct: Option<HashSet<Vec<NumberKey>>> = query.distinct.then(HashSet::new);

    if let (Some(order), Some(index)) = (&query.order_by, sort_index) {
        let mut sorted =
            SortedRows::with_offset(query.limit, query.offset.unwrap_or(0), order.descending)?;
        sorted.set_extra_directions(query.order_by_extra.iter().map(|term| term.descending));
        write_header(query, output, &projected.join("\t"))?;
        if query.limit == Some(0) {
            return Ok(());
        }
        while let Some(batch) = reader.next_batch()? {
            for row in 0..batch[0].len() {
                if !no_filters && !batch_matches(&batch, row, &comparisons) {
                    continue;
                }
                let key = number_at(&batch[index], row);
                // Defer secondary-key decoding and allocation until the primary
                // key actually has a chance to enter the bounded Top-K heap.
                // DISTINCT must still see every row before pruning.
                if !query.distinct
                    && sorted.would_discard(
                        key,
                        extra_sort_indices
                            .iter()
                            .map(|&index| number_at(&batch[index], row)),
                    )
                {
                    continue;
                }
                let extra_keys = extra_sort_indices
                    .iter()
                    .map(|&index| number_at(&batch[index], row))
                    .collect::<Vec<_>>();
                let values: Vec<Number> = projected_indices
                    .iter()
                    .map(|projected_index| number_at(&batch[*projected_index], row))
                    .collect();
                if let Some(seen) = distinct.as_mut() {
                    if !remember_distinct(seen, &values)? {
                        continue;
                    }
                }
                sorted.push_with_extra(key, extra_keys, values)?;
            }
        }
        return sorted.write(output);
    }

    let is_count = matches!(query.projection, Projection::Count);
    if is_count {
        write_header(query, output, "count")?;
    } else {
        write_header(query, output, &projected.join("\t"))?;
    }
    if query.limit == Some(0) {
        return Ok(());
    }

    let limit = query.limit.unwrap_or(usize::MAX);
    let mut emitted = 0usize;
    let offset = query.offset.unwrap_or(0);
    // OFFSET on an unsorted, unfiltered projection can bypass entire stored
    // granules. WHERE, DISTINCT, aggregate and ORDER BY must still scan rows.
    let mut skipped = if !is_count
        && !query.distinct
        && query.predicates.is_empty()
        && query.or_predicates.is_empty()
    {
        reader.skip_prefix_rows(offset)?
    } else {
        0
    };
    let mut matched = 0u64;
    while let Some(batch) = reader.next_batch()? {
        if is_count {
            matched += count_batch_matches(&batch, &comparisons) as u64;
            continue;
        }
        let rows = batch[0].len();
        for row in 0..rows {
            if !no_filters && !batch_matches(&batch, row, &comparisons) {
                continue;
            }
            if is_count {
                matched += 1;
            } else {
                let distinct_values = if let Some(seen) = distinct.as_mut() {
                    let values = projected_indices
                        .iter()
                        .map(|index| number_at(&batch[*index], row))
                        .collect::<Vec<_>>();
                    if !remember_distinct(seen, &values)? {
                        continue;
                    }
                    Some(values)
                } else {
                    None
                };
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                for (column_number, index) in projected_indices.iter().enumerate() {
                    if column_number != 0 {
                        write!(output, "\t")?;
                    }
                    let value = match &distinct_values {
                        Some(values) => values[column_number],
                        None => number_at(&batch[*index], row),
                    };
                    write!(output, "{value}")?;
                }
                writeln!(output)?;
                emitted += 1;
                if emitted == limit {
                    return Ok(());
                }
            }
        }
    }

    if is_count && query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{matched}")?;
    }
    Ok(())
}

const MAX_GROUPS: usize = 200_000;

// Select just the first N ordered groups without sorting all groups.
// The original index breaks ties in the same order as stable sort.
fn sorted_group_positions<T>(
    values: &[T],
    end: usize,
    cmp: impl Fn(&T, &T) -> Ordering,
) -> Vec<usize> {
    let mut positions = (0..values.len()).collect::<Vec<_>>();
    let compare = |left: &usize, right: &usize| {
        cmp(&values[*left], &values[*right]).then_with(|| left.cmp(right))
    };
    if end < positions.len() {
        positions.select_nth_unstable_by(end, &compare);
        positions.truncate(end);
    }
    positions.sort_unstable_by(compare);
    positions
}

#[derive(Default)]
struct GroupedCounts {
    by_key: HashMap<NumberKey, usize>,
    rows: Vec<(Number, u64)>,
}

impl GroupedCounts {
    fn push(&mut self, value: Number) -> Result<(), ExecutionError> {
        // Entry avoids hashing a new key twice (lookup followed by insert).
        match self.by_key.entry(NumberKey::from(value)) {
            Entry::Occupied(entry) => {
                let count = &mut self.rows[*entry.get()].1;
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
            }
            Entry::Vacant(entry) => {
                if self.rows.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 200000 groups".into(),
                    ));
                }
                entry.insert(self.rows.len());
                self.rows.push((value, 1));
            }
        }
        Ok(())
    }

    fn write<W: Write>(
        mut self,
        query: &Select,
        column: &str,
        output: &mut W,
    ) -> Result<(), ExecutionError> {
        if let Some(having) = &query.having {
            self.rows.retain(|(_, count)| {
                number_matches(
                    Number::Unsigned(u128::from(*count)),
                    Number::Unsigned(u128::from(having.value)),
                    having.operator,
                )
            });
        }
        let terms: Vec<_> = query.order_terms().collect();
        for order in &terms {
            if order.column != column && order.column != "count" {
                return Err(ExecutionError::Invalid(
                    "grouped ORDER BY must use the group key or count".into(),
                ));
            }
        }
        let compare = |(left_key, left_count): &(Number, u64),
                       (right_key, right_count): &(Number, u64)| {
            for order in &terms {
                let comparison = if order.column == "count" {
                    left_count.cmp(right_count)
                } else {
                    compare_numbers(*left_key, *right_key)
                };
                let comparison = if order.descending {
                    comparison.reverse()
                } else {
                    comparison
                };
                if comparison != Ordering::Equal {
                    return comparison;
                }
            }
            compare_numbers(*left_key, *right_key)
        };
        if !terms.is_empty() {
            if let Some(end) = query
                .limit
                .and_then(|limit| query.offset.unwrap_or(0).checked_add(limit))
                .filter(|&end| end < self.rows.len())
            {
                let positions = sorted_group_positions(&self.rows, end, compare);
                writeln!(
                    output,
                    "{column}\t{}",
                    query.alias.as_deref().unwrap_or("count")
                )?;
                for index in positions.into_iter().skip(query.offset.unwrap_or(0)) {
                    let (value, count) = self.rows[index];
                    writeln!(output, "{value}\t{count}")?;
                }
                return Ok(());
            }
            self.rows.sort_by(compare);
        }
        writeln!(
            output,
            "{column}\t{}",
            query.alias.as_deref().unwrap_or("count")
        )?;
        for (value, count) in self
            .rows
            .into_iter()
            .skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX))
        {
            writeln!(output, "{value}\t{count}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod grouped_topk_tests {
    use super::sorted_group_positions;

    #[test]
    fn keeps_stable_ties_and_offset_candidates() {
        let groups = [(10, 2), (11, 2), (12, 1), (13, 3), (14, 2)];
        let positions = sorted_group_positions(&groups, 3, |a, b| b.1.cmp(&a.1));
        assert_eq!(positions, vec![3, 0, 1]);
        let page = positions
            .into_iter()
            .skip(1)
            .map(|i| groups[i].0)
            .collect::<Vec<_>>();
        assert_eq!(page, vec![10, 11]);
    }

    #[test]
    fn handles_empty_page_and_full_sort() {
        let values = [3, 1, 2];
        assert!(sorted_group_positions(&values, 0, |a, b| a.cmp(b)).is_empty());
        assert_eq!(
            sorted_group_positions(&values, 3, |a, b| a.cmp(b)),
            vec![1, 2, 0]
        );
    }
}

fn execute_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    part: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if part.is_some() {
        return Err(ExecutionError::Invalid(
            "--part cannot be used with numbers(N)".into(),
        ));
    }
    // Protect large virtual sources from full scans while allowing exact
    // arithmetic aggregates and direct pages over generated unique keys.
    if rows > 10_000_000 {
        let scalar_count = matches!(
            query.projection,
            Projection::Count | Projection::CountColumn { .. } | Projection::CountDistinct { .. }
                | Projection::ApproxDistinct { .. }
        );
        let simple_result = query.order_by.is_none()
            && query.order_by_extra.is_empty()
            && query.having.is_none()
            && !query.distinct;
        let arithmetic_count = if query.predicates.is_empty() && query.or_predicates.is_empty() {
            true
        } else if scalar_count && simple_result {
            let compiled = compile_numbers_predicates(query)?;
            count_monotonic_numbers(&compiled, start, rows, step).is_some()
        } else {
            false
        };
        // Bounded pages are also safe when we can jump directly to the
        // requested generated values instead of scanning the full input.
        let bounded_page = matches!(query.projection, Projection::All | Projection::Columns(_))
            && query.limit.is_some_and(|limit| limit <= MAX_SORTED_ROWS)
            && query.having.is_none()
            && !query.distinct
            && query.order_by_extra.is_empty()
            && ((query.predicates.is_empty() && query.or_predicates.is_empty())
                || monotonic_range_indices(&compile_numbers_predicates(query)?, start, rows, step)
                    .is_some());
        // A GROUP BY number page has one row per key and can jump directly
        // to the requested offset without materializing earlier groups.
        let grouped_page = (matches!(
            &query.projection,
            Projection::GroupedCount { column } if column == "number"
        ) || matches!(
            &query.projection,
            Projection::GroupedAggregate { column, value_column, .. }
                if column == "number" && value_column == "number"
        )) && query.limit.is_some_and(|limit| limit <= MAX_SORTED_ROWS)
            && query.order_by.is_none()
            && simple_result
            && ((query.predicates.is_empty() && query.or_predicates.is_empty())
                || monotonic_range_indices(&compile_numbers_predicates(query)?, start, rows, step)
                    .is_some());
        // Only admit aggregates with proven O(1) or bounded sparse fast paths.
        // Other aggregate functions still require the normal scan bound.
        let direct_aggregate = if let Projection::Aggregate { function, column } = &query.projection
        {
            column == "number"
                && matches!(
                    function,
                    AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max
                )
                && simple_result
                && (if query.predicates.is_empty() && query.or_predicates.is_empty() {
                    true
                } else {
                    let compiled = compile_numbers_predicates(query)?;
                    monotonic_range_indices(&compiled, start, rows, step).is_some()
                        || sparse_generated_positions(&compiled, start, rows, step).is_some()
                        || excluded_generated_positions(&compiled, start, rows, step).is_some()
                })
        } else {
            false
        };
        if !(scalar_count && simple_result && arithmetic_count)
            && !bounded_page
            && !grouped_page
            && !direct_aggregate
        {
            return Err(ExecutionError::Invalid(
                "virtual inputs over 10000000 rows require an exact aggregate or bounded page query".into(),
            ));
        }
    }
    if let Projection::MultipleAggregates(items) = &query.projection {
        return multi_aggregate::execute_numbers(query, start, rows, step, items, output);
    }
    if let Projection::GroupedAggregates { column, items, aliases } = &query.projection {
        return grouped_generated::execute(query, start, rows, step, column, items, aliases, output);
    }
    if matches!(
        query.projection,
        Projection::GroupedAggregates { .. }
            | Projection::GroupedMultiAggregates { .. }
            | Projection::GroupedDistinctCount { .. }
        | Projection::GroupedMultiDistinctCount { .. }
    ) {
        return Err(ExecutionError::Invalid(
            "multiple aggregates require a stored table".into(),
        ));
    }
    if let Projection::CountColumn { column } = &query.projection {
        if column != "number" {
            return Err(ExecutionError::Invalid(
                "numbers(N) exposes only the number column".into(),
            ));
        }
        let comparisons = compile_numbers_predicates(query)?;
        write_header(query, output, "count(number)")?;
        if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            let count = if query.predicates.is_empty() && query.or_predicates.is_empty() {
                rows
            } else {
                count_monotonic_numbers(&comparisons, start, rows, step).unwrap_or_else(|| {
                    let mut count = 0u64;
                    for index in 0..rows {
                        let number = start + index * step;
                        if number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
                            count += 1;
                        }
                    }
                    count
                })
            };
            writeln!(output, "{count}")?;
        }
        return Ok(());
    }
    if let Projection::CountDistinct { column } | Projection::ApproxDistinct { column } = &query.projection {
        if column != "number" {
            return Err(ExecutionError::Invalid(
                "numbers(N) exposes only the number column".into(),
            ));
        }
        let comparisons = compile_numbers_predicates(query)?;
        let header = if matches!(query.projection, Projection::ApproxDistinct { .. }) {
            "uniq(number)"
        } else {
            "countDistinct(number)"
        };
        write_header(query, output, header)?;
        if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            // Generated numbers are unique, so unfiltered COUNT(DISTINCT) is their row count.
            let count = if query.predicates.is_empty() && query.or_predicates.is_empty() {
                rows
            } else {
                count_monotonic_numbers(&comparisons, start, rows, step).unwrap_or_else(|| {
                    let mut matched = 0u64;
                    for index in 0..rows {
                        let number = start + index * step;
                        if number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
                            matched += 1;
                        }
                    }
                    matched
                })
            };
            writeln!(output, "{count}")?;
        }
        return Ok(());
    }
    if let Projection::Aggregate { function, column } = &query.projection {
        return execute_aggregate_numbers(query, start, rows, step, *function, column, output);
    }
    if let Projection::GroupedCount { column } = &query.projection {
        return execute_grouped_numbers(query, start, rows, step, column, output);
    }
    if matches!(query.projection, Projection::GroupedAggregate { .. }) {
        return execute_grouped_aggregate_numbers(query, start, rows, step, output);
    }
    let projected: Vec<&str> = match &query.projection {
        Projection::All => vec!["number"],
        Projection::Columns(names) => {
            if names.iter().any(|name| name != "number") {
                return Err(ExecutionError::Invalid(
                    "numbers(N) exposes only the number column".into(),
                ));
            }
            names.iter().map(String::as_str).collect()
        }
        Projection::Count => Vec::new(),
        Projection::Aggregate { .. }
        | Projection::CountColumn { .. }
        | Projection::CountDistinct { .. }
        | Projection::ApproxDistinct { .. }
        | Projection::GroupedApproxDistinct { .. }
        | Projection::GroupedMultiApproxDistinct { .. }
        | Projection::GroupedCount { .. }
        | Projection::GroupedDistinctCount { .. }
        | Projection::GroupedMultiDistinctCount { .. }
        | Projection::GroupedMultiCount { .. }
        | Projection::GroupedAggregate { .. }
        | Projection::GroupedAggregates { .. }
        | Projection::GroupedMultiAggregates { .. }
        | Projection::MultipleAggregates(_)
        | Projection::StringLength { .. }
        | Projection::Literal(_)
        | Projection::Constants(_) => unreachable!(),
    };
    let comparisons = compile_numbers_predicates(query)?;
    let is_count = matches!(query.projection, Projection::Count);
    let mut sorting = query
        .order_by
        .as_ref()
        .map(|order| {
            if query.order_source_column(&order.column) != "number" {
                return Err(ExecutionError::Invalid(
                    "numbers(N) exposes only the number column".into(),
                ));
            }
            SortedRows::with_offset(query.limit, query.offset.unwrap_or(0), order.descending)
        })
        .transpose()?;
    if query
        .order_by_extra
        .iter()
        .any(|order| query.order_source_column(&order.column) != "number")
    {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    if is_count {
        write_header(query, output, "count")?;
        if query.predicates.is_empty() && query.or_predicates.is_empty() {
            if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{rows}")?;
            }
            return Ok(());
        }
        if sorting.is_none() && query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            if let Some(count) = count_monotonic_numbers(&comparisons, start, rows, step) {
                writeln!(output, "{count}")?;
                return Ok(());
            }
        }
    } else {
        write_header(query, output, &projected.join("\t"))?;
    }
    if query.limit == Some(0) {
        return Ok(());
    }

    // All generated values are unique and monotonic. With a simple range
    // predicate, both the first matching index and the page boundary are
    // known without visiting any of the preceding billion generated rows.
    if !is_count && (!query.predicates.is_empty() || !query.or_predicates.is_empty()) {
        if let Some(limit) = query.limit {
            if let Some((lower, upper)) = monotonic_range_indices(&comparisons, start, rows, step) {
                let available = upper - lower;
                let skip = (query.offset.unwrap_or(0) as u64).min(available);
                let emit = (limit as u64).min(available - skip);
                let descending = sorting.as_ref().is_some_and(SortedRows::descending);
                for position in skip..skip + emit {
                    let index = if descending {
                        upper - 1 - position
                    } else {
                        lower + position
                    };
                    let number = u128::from(start) + u128::from(index) * u128::from(step);
                    for column in 0..projected.len() {
                        if column > 0 {
                            output.write_all(b"\t")?;
                        }
                        write!(output, "{number}")?;
                    }
                    writeln!(output)?;
                }
                return Ok(());
            }
        }
    }

    // numbers() is already sorted and its values are unique. A filter-free
    // ORDER BY can page the generated range directly rather than filling a heap.
    let unfiltered_sort = if query.predicates.is_empty() && query.or_predicates.is_empty() {
        sorting.as_ref()
    } else {
        None
    };
    if let Some(sorted) = unfiltered_sort {
        if rows > MAX_SORTED_ROWS as u64 && sorted.requested() > MAX_SORTED_ROWS {
            return Err(ExecutionError::Invalid(format!(
                "ORDER BY requires LIMIT <= {MAX_SORTED_ROWS} for large results"
            )));
        }
        let total = usize::try_from(rows).unwrap_or(usize::MAX);
        let begin = sorted.offset().min(total);
        let end = sorted
            .offset()
            .saturating_add(query.limit.unwrap_or(usize::MAX))
            .min(total);
        for position in begin..end {
            let index = if sorted.descending() {
                rows - position as u64 - 1
            } else {
                position as u64
            };
            let number = start + index * step;
            for column in 0..projected.len() {
                if column != 0 {
                    write!(output, "\t")?;
                }
                write!(output, "{number}")?;
            }
            writeln!(output)?;
        }
        return Ok(());
    }

    // A filtered generated sequence is still ordered by number. For a
    // bounded ORDER BY page, scan in the requested direction and stop after
    // the page is full instead of building a heap from every matching row.
    // Keep the existing bounded-sort limit and unbounded-sort behavior.
    if !is_count {
        if let (Some(sorted), Some(limit)) = (sorting.as_ref(), query.limit) {
            if sorted.requested() <= MAX_SORTED_ROWS {
                let mut remaining_offset = sorted.offset();
                let mut emitted = 0usize;
                for position in 0..rows {
                    let index = if sorted.descending() {
                        rows - 1 - position
                    } else {
                        position
                    };
                    let number = start + index * step;
                    if !number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
                        continue;
                    }
                    if remaining_offset > 0 {
                        remaining_offset -= 1;
                        continue;
                    }
                    for column in 0..projected.len() {
                        if column > 0 {
                            output.write_all(b"\t")?;
                        }
                        write!(output, "{number}")?;
                    }
                    writeln!(output)?;
                    emitted += 1;
                    if emitted == limit {
                        break;
                    }
                }
                return Ok(());
            }
        }
    }

    let mut matched = 0u64;
    let mut emitted = 0usize;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(usize::MAX);
    // Generated rows are unique and ordered. A filter-free scan can start at OFFSET.
    let fast_skip = if !is_count
        && sorting.is_none()
        && query.predicates.is_empty()
        && query.or_predicates.is_empty()
    {
        offset.min(rows as usize)
    } else {
        0
    };
    let mut skipped = fast_skip;
    for index in fast_skip as u64..rows {
        let number = start + index * step;
        if !number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
            continue;
        }
        if let Some(sorted) = sorting.as_mut() {
            let value = Number::Unsigned(u128::from(number));
            sorted.push(value, vec![value; projected.len()])?;
            continue;
        }
        if is_count {
            matched += 1;
        } else {
            if skipped < offset {
                skipped += 1;
                continue;
            }
            for (index, _) in projected.iter().enumerate() {
                if index != 0 {
                    write!(output, "\t")?;
                }
                write!(output, "{number}")?;
            }
            writeln!(output)?;
            emitted += 1;
            if emitted == limit {
                break;
            }
        }
    }
    if let Some(sorted) = sorting {
        return sorted.write(output);
    }
    if is_count && query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{matched}")?;
    }
    Ok(())
}

struct AggregateState {
    function: AggregateFunction,
    value: Option<Number>,
    max_value: Option<Number>,
    mean: f64,
    squared_deviations: f64,
    count: u64,
    median_values: Vec<Number>,
}

impl AggregateState {
    fn new(function: AggregateFunction) -> Self {
        Self {
            function,
            value: None,
            max_value: None,
            mean: 0.0,
            squared_deviations: 0.0,
            count: 0,
            median_values: Vec::new(),
        }
    }

    /// Count a non-NULL grouped row without going through numeric value dispatch.
    fn increment_count(&mut self) -> Result<(), ExecutionError> {
        debug_assert_eq!(self.function, AggregateFunction::Count);
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("aggregate row count overflow".into()))?;
        Ok(())
    }

    fn update(&mut self, value: Number) -> Result<(), ExecutionError> {
        let next_count = self
            .count
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("aggregate row count overflow".into()))?;
        match self.function {
            AggregateFunction::GroupBitXor => {
                let combined = match (self.value, value) {
                    (None, value @ (Number::Unsigned(_) | Number::Signed(_))) => value,
                    (Some(Number::Unsigned(a)), Number::Unsigned(b)) => Number::Unsigned(a ^ b),
                    (Some(Number::Signed(a)), Number::Signed(b)) => Number::Signed(a ^ b),
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "groupBitXor requires integer values".into(),
                        ));
                    }
                };
                self.value = Some(combined);
            }
            AggregateFunction::GroupBitOr => {
                let combined = match (self.value, value) {
                    (None, value @ (Number::Unsigned(_) | Number::Signed(_))) => value,
                    (Some(Number::Unsigned(a)), Number::Unsigned(b)) => Number::Unsigned(a | b),
                    (Some(Number::Signed(a)), Number::Signed(b)) => Number::Signed(a | b),
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "groupBitOr requires integer values".into(),
                        ));
                    }
                };
                self.value = Some(combined);
            }
            AggregateFunction::GroupBitAnd => {
                let combined = match (self.value, value) {
                    (None, value @ (Number::Unsigned(_) | Number::Signed(_))) => value,
                    (Some(Number::Unsigned(a)), Number::Unsigned(b)) => Number::Unsigned(a & b),
                    (Some(Number::Signed(a)), Number::Signed(b)) => Number::Signed(a & b),
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "groupBitAnd requires integer values".into(),
                        ));
                    }
                };
                self.value = Some(combined);
            }
            AggregateFunction::AnyLast => {
                self.value = Some(value);
            }
            AggregateFunction::Any => {
                if self.value.is_none() {
                    self.value = Some(value);
                }
            }
            AggregateFunction::Count => {}
            AggregateFunction::CountIf => {
                let truthy = match value {
                    Number::Unsigned(n) => n != 0,
                    Number::Signed(n) => n != 0,
                    Number::Float(n) => n != 0.0,
                };
                if truthy {
                    let current = match self.value {
                        Some(Number::Unsigned(n)) => n,
                        _ => 0,
                    };
                    self.value = Some(Number::Unsigned(current + 1));
                }
            }
            AggregateFunction::Sum | AggregateFunction::SumSquares => {
                let value = if self.function == AggregateFunction::SumSquares {
                    match value {
                        Number::Unsigned(n) => {
                            Number::Unsigned(n.checked_mul(n).ok_or_else(|| {
                                ExecutionError::Invalid("unsigned SUMSQUARES overflow".into())
                            })?)
                        }
                        Number::Signed(n) => Number::Signed(n.checked_mul(n).ok_or_else(|| {
                            ExecutionError::Invalid("signed SUMSQUARES overflow".into())
                        })?),
                        Number::Float(n) => Number::Float(n * n),
                    }
                } else {
                    value
                };
                let sum = match (self.value, value) {
                    (None, value) => value,
                    (Some(Number::Unsigned(a)), Number::Unsigned(b)) => {
                        Number::Unsigned(a.checked_add(b).ok_or_else(|| {
                            ExecutionError::Invalid("unsigned SUM overflow".into())
                        })?)
                    }
                    (Some(Number::Signed(a)), Number::Signed(b)) => Number::Signed(
                        a.checked_add(b)
                            .ok_or_else(|| ExecutionError::Invalid("signed SUM overflow".into()))?,
                    ),
                    (Some(Number::Float(a)), Number::Float(b)) => Number::Float(a + b),
                    _ => {
                        return Err(ExecutionError::Invalid("incompatible types in SUM".into()));
                    }
                };
                self.value = Some(sum);
            }
            AggregateFunction::Product => {
                let product = match (self.value, value) {
                    (None, value) => value,
                    (Some(Number::Unsigned(a)), Number::Unsigned(b)) => {
                        Number::Unsigned(a.checked_mul(b).ok_or_else(|| {
                            ExecutionError::Invalid("unsigned PRODUCT overflow".into())
                        })?)
                    }
                    (Some(Number::Signed(a)), Number::Signed(b)) => {
                        Number::Signed(a.checked_mul(b).ok_or_else(|| {
                            ExecutionError::Invalid("signed PRODUCT overflow".into())
                        })?)
                    }
                    (Some(Number::Float(a)), Number::Float(b)) => Number::Float(a * b),
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "incompatible types in PRODUCT".into(),
                        ));
                    }
                };
                self.value = Some(product);
            }
            AggregateFunction::Avg
            | AggregateFunction::VarPop
            | AggregateFunction::StddevPop
            | AggregateFunction::VarSamp
            | AggregateFunction::StddevSamp => {
                let numeric = match value {
                    Number::Unsigned(n) => n as f64,
                    Number::Signed(n) => n as f64,
                    Number::Float(n) => n,
                };
                let difference = numeric - self.mean;
                self.mean += difference / next_count as f64;
                if matches!(
                    self.function,
                    AggregateFunction::VarPop
                        | AggregateFunction::StddevPop
                        | AggregateFunction::VarSamp
                        | AggregateFunction::StddevSamp
                ) {
                    // Welford's algorithm avoids subtracting large squared sums.
                    self.squared_deviations += difference * (numeric - self.mean);
                }
            }
            AggregateFunction::Rms => {
                let numeric = match value {
                    Number::Unsigned(n) => n as f64,
                    Number::Signed(n) => n as f64,
                    Number::Float(n) => n,
                };
                let total = next_count as f64;
                let old_weight = (self.count as f64 / total).sqrt();
                self.mean = (self.mean * old_weight).hypot(numeric / total.sqrt());
            }
            AggregateFunction::Median => {
                if self.median_values.len() >= MAX_MEDIAN_ROWS {
                    return Err(ExecutionError::Invalid(
                        "median supports at most 100000 matching rows".into(),
                    ));
                }
                self.median_values.push(value);
            }
            AggregateFunction::Range => {
                let lower = self.value.map_or(value, |current| {
                    if compare_numbers(value, current).is_lt() {
                        value
                    } else {
                        current
                    }
                });
                let upper = self.max_value.map_or(value, |current| {
                    if compare_numbers(value, current).is_gt() {
                        value
                    } else {
                        current
                    }
                });
                self.value = Some(lower);
                self.max_value = Some(upper);
            }
            AggregateFunction::Min | AggregateFunction::Max => {
                let replace = self.value.is_none_or(|current| {
                    let order = compare_numbers(value, current);
                    match self.function {
                        AggregateFunction::Min => order.is_lt(),
                        AggregateFunction::Max => order.is_gt(),
                        _ => false,
                    }
                });
                if replace {
                    self.value = Some(value);
                }
            }
        }
        self.count = next_count;
        Ok(())
    }

    /// Compare numeric aggregates without rounding through their TSV rendering.
    fn passes_having(&self, having: &crate::Having) -> bool {
        let typed::Cell::Numeric(value) = self.sort_cell() else {
            // SQL HAVING removes groups whose aggregate is NULL.
            return false;
        };
        let threshold = match value {
            Number::Unsigned(_) => Number::Unsigned(u128::from(having.value)),
            Number::Signed(_) => Number::Signed(i128::from(having.value)),
            Number::Float(_) => Number::Float(having.value as f64),
        };
        number_matches(value, threshold, having.operator)
    }

    fn median_number(&self) -> Option<Number> {
        if self.median_values.is_empty() {
            return None;
        }
        let mut values = self.median_values.clone();
        values.sort_unstable_by(|left, right| compare_numbers(*left, *right));
        let middle = values.len() / 2;
        if values.len() % 2 == 1 {
            return Some(values[middle]);
        }
        let as_float = |value: Number| match value {
            Number::Unsigned(value) => value as f64,
            Number::Signed(value) => value as f64,
            Number::Float(value) => value,
        };
        Some(Number::Float(
            as_float(values[middle - 1]) / 2.0 + as_float(values[middle]) / 2.0
        ))
    }

    /// Exact comparisons for grouped analytics, including statistical HAVING.
    fn sort_cell(&self) -> typed::Cell {
        let nonempty = self.count > 0;
        let sample = self.count > 1;
        let value = match self.function {
            AggregateFunction::Count => Some(Number::Unsigned(u128::from(self.count))),
            AggregateFunction::CountIf => Some(self.value.unwrap_or(Number::Unsigned(0))),
            AggregateFunction::Sum
            | AggregateFunction::Product
            | AggregateFunction::SumSquares
            | AggregateFunction::Min
            | AggregateFunction::Max
            | AggregateFunction::Any
            | AggregateFunction::AnyLast
            | AggregateFunction::GroupBitAnd
            | AggregateFunction::GroupBitOr
            | AggregateFunction::GroupBitXor => self.value,
            AggregateFunction::Avg | AggregateFunction::Rms if nonempty =>
                Some(Number::Float(self.mean)),
            AggregateFunction::VarPop if nonempty =>
                Some(Number::Float(self.squared_deviations / self.count as f64)),
            AggregateFunction::StddevPop if nonempty =>
                Some(Number::Float((self.squared_deviations / self.count as f64).sqrt())),
            AggregateFunction::VarSamp if sample =>
                Some(Number::Float(self.squared_deviations / (self.count - 1) as f64)),
            AggregateFunction::StddevSamp if sample =>
                Some(Number::Float((self.squared_deviations / (self.count - 1) as f64).sqrt())),
            AggregateFunction::Median => self.median_number(),
            AggregateFunction::Range => match (self.value, self.max_value) {
                (Some(Number::Unsigned(low)), Some(Number::Unsigned(high))) =>
                    Some(Number::Unsigned(high - low)),
                (Some(Number::Signed(low)), Some(Number::Signed(high))) =>
                    Some(Number::Unsigned(high.abs_diff(low))),
                (Some(Number::Float(low)), Some(Number::Float(high))) =>
                    Some(Number::Float(high - low)),
                _ => None,
            },
            _ => None,
        };
        value.map_or(typed::Cell::Null, typed::Cell::Numeric)
    }

    fn render(&self) -> String {
        if self.function == AggregateFunction::Count {
            return self.count.to_string();
        }
        if self.function == AggregateFunction::CountIf {
            return self
                .value
                .map_or_else(|| "0".into(), |count| count.to_string());
        }
        if self.count == 0
            || (self.count == 1
                && matches!(
                    self.function,
                    AggregateFunction::VarSamp | AggregateFunction::StddevSamp
                ))
        {
            return "NULL".into();
        }
        match self.function {
            AggregateFunction::Avg => self.mean.to_string(),
            AggregateFunction::Rms => self.mean.to_string(),
            AggregateFunction::VarPop => (self.squared_deviations / self.count as f64).to_string(),
            AggregateFunction::StddevPop => (self.squared_deviations / self.count as f64)
                .sqrt()
                .to_string(),
            AggregateFunction::VarSamp => {
                (self.squared_deviations / (self.count - 1) as f64).to_string()
            }
            AggregateFunction::StddevSamp => (self.squared_deviations / (self.count - 1) as f64)
                .sqrt()
                .to_string(),
            AggregateFunction::Range => match (self.value, self.max_value) {
                (Some(Number::Unsigned(low)), Some(Number::Unsigned(high))) => {
                    (high - low).to_string()
                }
                (Some(Number::Signed(low)), Some(Number::Signed(high))) => {
                    high.abs_diff(low).to_string()
                }
                (Some(Number::Float(low)), Some(Number::Float(high))) => (high - low).to_string(),
                _ => unreachable!("nonempty RANGE has both extrema"),
            },
            AggregateFunction::Median => {
                let mut values = self.median_values.clone();
                values.sort_unstable_by(|left, right| compare_numbers(*left, *right));
                let middle = values.len() / 2;
                if values.len() & 1 == 1 {
                    values[middle].to_string()
                } else {
                    // Even-length interpolation uses Float64, as AVG already does.
                    // Divide before adding to avoid overflow for very large values.
                    let as_f64 = |number: Number| match number {
                        Number::Unsigned(value) => value as f64,
                        Number::Signed(value) => value as f64,
                        Number::Float(value) => value,
                    };
                    (as_f64(values[middle - 1]) / 2.0 + as_f64(values[middle]) / 2.0).to_string()
                }
            }
            _ => self
                .value
                .map_or_else(String::new, |value| value.to_string()),
        }
    }
}

fn compare_numbers(left: Number, right: Number) -> Ordering {
    match (left, right) {
        (Number::Unsigned(a), Number::Unsigned(b)) => a.cmp(&b),
        (Number::Signed(a), Number::Signed(b)) => a.cmp(&b),
        (Number::Float(a), Number::Float(b)) => a.total_cmp(&b),
        (Number::Unsigned(_), _) => Ordering::Less,
        (_, Number::Unsigned(_)) => Ordering::Greater,
        (Number::Signed(_), _) => Ordering::Less,
        (_, Number::Signed(_)) => Ordering::Greater,
    }
}

fn number_at(column: &NumericColumn, row: usize) -> Number {
    match column {
        NumericColumn::UInt8(values) => Number::Unsigned(u128::from(values[row])),
        NumericColumn::UInt16(values) => Number::Unsigned(u128::from(values[row])),
        NumericColumn::UInt32(values) => Number::Unsigned(u128::from(values[row])),
        NumericColumn::UInt64(values) => Number::Unsigned(u128::from(values[row])),
        NumericColumn::UInt128(values) => Number::Unsigned(values[row]),
        NumericColumn::Int8(values) => Number::Signed(i128::from(values[row])),
        NumericColumn::Int16(values) => Number::Signed(i128::from(values[row])),
        NumericColumn::Int32(values) => Number::Signed(i128::from(values[row])),
        NumericColumn::Int64(values) => Number::Signed(i128::from(values[row])),
        NumericColumn::Int128(values) => Number::Signed(values[row]),
        NumericColumn::Float32(values) => Number::Float(f64::from(values[row])),
        NumericColumn::Float64(values) => Number::Float(values[row]),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod parallel_sum_tests {
    use super::*;

    #[test]
    fn partial_sum_reduction_handles_null_signed_unsigned_and_overflow() {
        let signed = add_integer_partial(None, Some(Number::Signed(-7))).unwrap();
        let signed = add_integer_partial(signed, Some(Number::Signed(10))).unwrap();
        assert_eq!(signed.unwrap().to_string(), "3");
        assert_eq!(
            add_integer_partial(signed, None)
                .unwrap()
                .unwrap()
                .to_string(),
            "3"
        );

        let unsigned = add_integer_partial(
            Some(Number::Unsigned(u128::MAX - 1)),
            Some(Number::Unsigned(1)),
        )
        .unwrap();
        assert_eq!(unsigned.unwrap().to_string(), u128::MAX.to_string());
        assert!(add_integer_partial(unsigned, Some(Number::Unsigned(1))).is_err());
        assert!(
            add_integer_partial(Some(Number::Signed(i128::MAX)), Some(Number::Signed(1)),).is_err()
        );
        assert!(add_integer_partial(Some(Number::Signed(1)), Some(Number::Unsigned(1)),).is_err());
    }
}

#[cfg(test)]
mod numeric_distinct_capacity_tests {
    use super::*;

    #[test]
    fn accepts_more_than_old_distinct_limit_and_rejects_new_excess() {
        let mut seen = HashSet::new();
        for value in 0..MAX_DISTINCT_ROWS {
            assert!(remember_distinct(&mut seen, &[Number::Unsigned(value as u128)]).unwrap());
        }
        assert!(!remember_distinct(&mut seen, &[Number::Unsigned(0)]).unwrap());
        assert!(
            remember_distinct(&mut seen, &[Number::Unsigned(MAX_DISTINCT_ROWS as u128)]).is_err()
        );
    }
}

#[cfg(test)]
mod numeric_group_capacity_tests {
    use super::*;

    #[test]
    fn accepts_more_than_old_group_count_limit() {
        let mut groups = GroupedCounts::default();
        for value in 0..100_001u64 {
            groups.push(Number::Unsigned(u128::from(value))).unwrap();
        }
        assert_eq!(groups.rows.len(), 100_001);
    }
}
