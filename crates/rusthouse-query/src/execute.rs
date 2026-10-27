use crate::{AggregateFunction, Projection, Select, Source};
use rusthouse_storage::{NumericColumn, NumericQueryReader, PartError, inspect_query_source};
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::io::{self, BufWriter, Write};
use std::path::Path;

mod join;
mod predicates;
mod sort;
mod typed;
pub use join::execute_join;
use sort::{MAX_SORTED_ROWS, SortedRows};
mod typed_aggregate;
#[cfg(test)]
use predicates::{CompiledTest, compile_in_test};
use predicates::{
    batch_matches, compile_numbers_predicates, compile_part_where, count_batch_matches,
    number_matches, number_matches_all, parse_numeric,
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

const MAX_DISTINCT_ROWS: usize = 100_000;
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
            "DISTINCT supports at most 100000 unique rows".into(),
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

/// Evaluate filtered COUNT independently on immutable parts, then reduce the
/// partial counts. Skip small snapshots where thread startup costs dominate.
/// An unfiltered COUNT already has a cheaper metadata-only path.
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
    if parts.len() < MIN_PARALLEL_PARTS || workers < 2 {
        return Ok(None);
    }
    // Validate all part schemas before executing any partial count. A mismatched
    // or incomplete snapshot must never produce a plausible partial answer.
    inspect_query_source(source)?;
    let chunk_size = parts.len().div_ceil(workers);
    let total = std::thread::scope(|scope| -> Result<u64, ExecutionError> {
        let handles = parts
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || -> Result<u64, ExecutionError> {
                    let mut sum = 0u64;
                    let mut buffer = Vec::new();
                    for part in chunk {
                        buffer.clear();
                        // A single-part source avoids re-entering this parallel path.
                        execute(query, Some(part), &mut buffer)?;
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
    if let Some(count) = parallel_filtered_count(query, part)? {
        write_header(query, output, "count")?;
        writeln!(output, "{count}")?;
        return Ok(());
    }
    if matches!(query.projection, Projection::GroupedAggregate { .. }) {
        return typed_aggregate::execute_grouped_aggregate(query, part, output);
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
        | Projection::GroupedCount { .. }
        | Projection::GroupedAggregate { .. }
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
    let mut reader = NumericQueryReader::open(part, &scan_columns)?;
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
                if !batch_matches(&batch, row, &comparisons) {
                    continue;
                }
                let values: Vec<Number> = projected_indices
                    .iter()
                    .map(|projected_index| number_at(&batch[*projected_index], row))
                    .collect();
                if let Some(seen) = distinct.as_mut() {
                    if !remember_distinct(seen, &values)? {
                        continue;
                    }
                }
                let extra_keys = extra_sort_indices
                    .iter()
                    .map(|&index| number_at(&batch[index], row))
                    .collect();
                sorted.push_with_extra(number_at(&batch[index], row), extra_keys, values)?;
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
            if !batch_matches(&batch, row, &comparisons) {
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

const MAX_GROUPS: usize = 100_000;

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
                        "GROUP BY supports at most 100000 groups".into(),
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
        if !terms.is_empty() {
            self.rows
                .sort_by(|(left_key, left_count), (right_key, right_count)| {
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
                });
        }
        writeln!(output, "{column}\tcount")?;
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

fn execute_grouped_part<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut scan_columns = vec![column.to_owned()];
    let comparisons = compile_part_where(query, &mut scan_columns, |column_name, literal| {
        let metadata_column = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == column_name)
            .ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown WHERE column: {column_name}"))
            })?;
        parse_numeric(&metadata_column.data_type, literal)
    })?;
    let mut reader = NumericQueryReader::open(part, &scan_columns)?;
    let mut groups = GroupedCounts::default();
    if query.limit == Some(0) {
        return groups.write(query, column, output);
    }
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if batch_matches(&batch, row, &comparisons) {
                groups.push(number_at(&batch[0], row))?;
            }
        }
    }
    groups.write(query, column, output)
}

fn execute_grouped_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if column != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    let comparisons = compile_numbers_predicates(query)?;
    let mut groups = GroupedCounts::default();
    if query.limit == Some(0) {
        return groups.write(query, column, output);
    }
    for index in 0..rows {
        let number = start + index * step;
        let value = Number::Unsigned(u128::from(number));
        if number_matches_all(value, &comparisons) {
            groups.push(value)?;
        }
    }
    groups.write(query, column, output)
}

fn execute_grouped_aggregate_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::GroupedAggregate {
        column,
        function,
        value_column,
    } = &query.projection
    else {
        unreachable!("grouped aggregate source requires a grouped projection")
    };
    if column != "number" || value_column != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    let comparisons = compile_numbers_predicates(query)?;
    writeln!(output, "{column}\t{}({value_column})", function.name())?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let mut by_key = HashMap::<NumberKey, usize>::new();
    let mut groups = Vec::<(Number, AggregateState)>::new();
    for index in 0..rows {
        let number = start + index * step;
        let value = Number::Unsigned(u128::from(number));
        if !number_matches_all(value, &comparisons) {
            continue;
        }
        match by_key.entry(NumberKey::from(value)) {
            Entry::Occupied(entry) => groups[*entry.get()].1.update(value)?,
            Entry::Vacant(entry) => {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 100000 groups".into(),
                    ));
                }
                entry.insert(groups.len());
                let mut state = AggregateState::new(*function);
                state.update(value)?;
                groups.push((value, state));
            }
        }
    }
    if let Some(order) = &query.order_by {
        groups.sort_by(|(left, _), (right, _)| {
            let ordering = compare_numbers(*left, *right);
            if order.descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }
    for (key, state) in groups
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        writeln!(output, "{key}\t{}", state.render())?;
    }
    Ok(())
}

fn execute_count_column_part<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut scan_columns = vec![column.to_owned()];
    let comparisons = compile_part_where(query, &mut scan_columns, |name, literal| {
        let col = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
        parse_numeric(&col.data_type, literal)
    })?;
    let mut reader = NumericQueryReader::open(part, &scan_columns)?;
    write_header(query, output, &format!("count({column})"))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let mut count = 0u64;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if batch_matches(&batch, row, &comparisons) {
                count = count
                    .checked_add(1)
                    .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{count}")?;
    }
    Ok(())
}

fn execute_count_distinct_part<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut scan_columns = vec![column.to_owned()];
    let comparisons = compile_part_where(query, &mut scan_columns, |column_name, literal| {
        let metadata_column = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == column_name)
            .ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown WHERE column: {column_name}"))
            })?;
        parse_numeric(&metadata_column.data_type, literal)
    })?;
    let mut reader = NumericQueryReader::open(part, &scan_columns)?;
    write_header(query, output, &format!("countDistinct({column})"))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let mut seen = HashSet::new();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !batch_matches(&batch, row, &comparisons) {
                continue;
            }
            let key = NumberKey::from(number_at(&batch[0], row));
            if seen.len() >= MAX_DISTINCT_ROWS && !seen.contains(&key) {
                return Err(ExecutionError::Invalid(
                    "COUNT(DISTINCT) supports at most 100000 unique values".into(),
                ));
            }
            seen.insert(key);
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{}", seen.len())?;
    }
    Ok(())
}

fn execute_aggregate_part<W: Write>(
    query: &Select,
    part: &Path,
    function: AggregateFunction,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut scan_columns = vec![column.to_owned()];
    let comparisons = compile_part_where(query, &mut scan_columns, |column_name, literal| {
        let metadata_column = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == column_name)
            .ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown WHERE column: {column_name}"))
            })?;
        parse_numeric(&metadata_column.data_type, literal)
    })?;
    let mut reader = NumericQueryReader::open(part, &scan_columns)?;
    write_header(query, output, &format!("{}({column})", function.name()))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let mut state = AggregateState::new(function);
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !batch_matches(&batch, row, &comparisons) {
                continue;
            }
            state.update(number_at(&batch[0], row))?;
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{}", state.render())?;
    }
    Ok(())
}

fn execute_aggregate_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    function: AggregateFunction,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if column != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    let comparisons = compile_numbers_predicates(query)?;
    write_header(query, output, &format!("{}(number)", function.name()))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    // For an unfiltered arithmetic progression, these aggregates are exact
    // without visiting every row. Keep AVG iterative to preserve its rounding.
    if query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && matches!(
            function,
            AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max
        )
    {
        if query.offset.unwrap_or(0) == 0 {
            let result = if rows == 0 {
                "NULL".to_string()
            } else {
                let first = u128::from(start);
                let last = first + u128::from(rows - 1) * u128::from(step);
                match function {
                    AggregateFunction::Sum => ((first + last) * u128::from(rows) / 2).to_string(),
                    AggregateFunction::Min => first.to_string(),
                    AggregateFunction::Max => last.to_string(),
                    _ => unreachable!(),
                }
            };
            writeln!(output, "{result}")?;
        }
        return Ok(());
    }
    let mut state = AggregateState::new(function);
    for index in 0..rows {
        let number = start + index * step;
        let value = Number::Unsigned(u128::from(number));
        if !number_matches_all(value, &comparisons) {
            continue;
        }
        state.update(value)?;
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{}", state.render())?;
    }
    Ok(())
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
                let mut count = 0u64;
                for index in 0..rows {
                    let number = start + index * step;
                    if number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
                        count += 1;
                    }
                }
                count
            };
            writeln!(output, "{count}")?;
        }
        return Ok(());
    }
    if let Projection::CountDistinct { column } = &query.projection {
        if column != "number" {
            return Err(ExecutionError::Invalid(
                "numbers(N) exposes only the number column".into(),
            ));
        }
        let comparisons = compile_numbers_predicates(query)?;
        write_header(query, output, "countDistinct(number)")?;
        if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            // Generated numbers are unique, so unfiltered COUNT(DISTINCT) is their row count.
            let count = if query.predicates.is_empty() && query.or_predicates.is_empty() {
                rows
            } else {
                let mut matched = 0u64;
                for index in 0..rows {
                    let number = start + index * step;
                    if number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
                        matched += 1;
                    }
                }
                matched
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
        | Projection::GroupedCount { .. }
        | Projection::GroupedAggregate { .. }
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
    } else {
        write_header(query, output, &projected.join("\t"))?;
    }
    if query.limit == Some(0) {
        return Ok(());
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
            return Err(ExecutionError::Invalid(
                "ORDER BY requires LIMIT <= 100000 for large results".into(),
            ));
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

    fn update(&mut self, value: Number) -> Result<(), ExecutionError> {
        let next_count = self
            .count
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("aggregate row count overflow".into()))?;
        match self.function {
            AggregateFunction::Any => {
                if self.value.is_none() {
                    self.value = Some(value);
                }
            }
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

    fn render(&self) -> String {
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
