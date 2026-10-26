use crate::{AggregateFunction, Operator, Predicate, Projection, Select, Source};
use rusthouse_storage::{NumericColumn, NumericPartReader, PartError, inspect_part};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::io::{self, BufWriter, Write};
use std::path::Path;

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

fn execute_into<W: Write>(
    query: &Select,
    part: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if let Projection::Literal(value) = query.projection {
        writeln!(output, "{value}")?;
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
    if let Projection::CountDistinct { column } = &query.projection {
        return execute_count_distinct_part(query, part, column, output);
    }
    if let Projection::Aggregate { function, column } = &query.projection {
        return execute_aggregate_part(query, part, *function, column, output);
    }
    if let Projection::GroupedCount { column } = &query.projection {
        return execute_grouped_part(query, part, column, output);
    }
    let metadata = inspect_part(part)?;

    if matches!(query.projection, Projection::Count)
        && query.predicates.is_empty()
        && query.or_predicates.is_empty()
    {
        writeln!(output, "count")?;
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
        | Projection::CountDistinct { .. }
        | Projection::GroupedCount { .. }
        | Projection::Literal(_) => unreachable!(),
    };
    if query.distinct
        && query
            .order_by
            .as_ref()
            .is_some_and(|order| !projected.iter().any(|column| column == &order.column))
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
        if let Some(index) = scan_columns
            .iter()
            .position(|column| column == &order.column)
        {
            index
        } else {
            scan_columns.push(order.column.clone());
            scan_columns.len() - 1
        }
    });
    let mut reader = NumericPartReader::open(part, &scan_columns)?;
    let mut distinct: Option<HashSet<Vec<NumberKey>>> = query.distinct.then(HashSet::new);

    if let (Some(order), Some(index)) = (&query.order_by, sort_index) {
        let mut sorted =
            SortedRows::with_offset(query.limit, query.offset.unwrap_or(0), order.descending)?;
        writeln!(output, "{}", projected.join("\t"))?;
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
                sorted.push(number_at(&batch[index], row), values)?;
            }
        }
        return sorted.write(output);
    }

    let is_count = matches!(query.projection, Projection::Count);
    if is_count {
        writeln!(output, "count")?;
    } else {
        writeln!(output, "{}", projected.join("\t"))?;
    }
    if query.limit == Some(0) {
        return Ok(());
    }

    let limit = query.limit.unwrap_or(usize::MAX);
    let mut emitted = 0usize;
    let mut skipped = 0usize;
    let offset = query.offset.unwrap_or(0);
    let mut matched = 0u64;
    while let Some(batch) = reader.next_batch()? {
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
        let key = NumberKey::from(value);
        if let Some(&index) = self.by_key.get(&key) {
            self.rows[index].1 = self.rows[index]
                .1
                .checked_add(1)
                .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
        } else {
            if self.rows.len() >= MAX_GROUPS {
                return Err(ExecutionError::Invalid(
                    "GROUP BY supports at most 100000 groups".into(),
                ));
            }
            self.by_key.insert(key, self.rows.len());
            self.rows.push((value, 1));
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
        if let Some(order) = &query.order_by {
            if order.column != column && order.column != "count" {
                return Err(ExecutionError::Invalid(
                    "grouped ORDER BY must use the group key or count".into(),
                ));
            }
            self.rows
                .sort_by(|(left_key, left_count), (right_key, right_count)| {
                    let ordering = if order.column == "count" {
                        left_count.cmp(right_count)
                    } else {
                        compare_numbers(*left_key, *right_key)
                    };
                    let ordering = if order.descending {
                        ordering.reverse()
                    } else {
                        ordering
                    };
                    ordering.then_with(|| compare_numbers(*left_key, *right_key))
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
    let metadata = inspect_part(part)?;
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
    let mut reader = NumericPartReader::open(part, &scan_columns)?;
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

fn execute_count_distinct_part<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_part(part)?;
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
    let mut reader = NumericPartReader::open(part, &scan_columns)?;
    writeln!(output, "countDistinct({column})")?;
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
    let metadata = inspect_part(part)?;
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
    let mut reader = NumericPartReader::open(part, &scan_columns)?;
    writeln!(output, "{}({column})", function.name())?;
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
    writeln!(output, "{}(number)", function.name())?;
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
                    AggregateFunction::Avg => unreachable!(),
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
    if let Projection::CountDistinct { column } = &query.projection {
        if column != "number" {
            return Err(ExecutionError::Invalid(
                "numbers(N) exposes only the number column".into(),
            ));
        }
        let comparisons = compile_numbers_predicates(query)?;
        writeln!(output, "countDistinct(number)")?;
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
        | Projection::CountDistinct { .. }
        | Projection::GroupedCount { .. }
        | Projection::Literal(_) => unreachable!(),
    };
    let comparisons = compile_numbers_predicates(query)?;
    let is_count = matches!(query.projection, Projection::Count);
    let mut sorting = query
        .order_by
        .as_ref()
        .map(|order| {
            if order.column != "number" {
                return Err(ExecutionError::Invalid(
                    "numbers(N) exposes only the number column".into(),
                ));
            }
            SortedRows::with_offset(query.limit, query.offset.unwrap_or(0), order.descending)
        })
        .transpose()?;
    if is_count {
        writeln!(output, "count")?;
        if query.predicates.is_empty() && query.or_predicates.is_empty() {
            if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{rows}")?;
            }
            return Ok(());
        }
    } else {
        writeln!(output, "{}", projected.join("\t"))?;
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
        if rows > MAX_SORTED_ROWS as u64 && sorted.requested > MAX_SORTED_ROWS {
            return Err(ExecutionError::Invalid(
                "ORDER BY requires LIMIT <= 100000 for large results".into(),
            ));
        }
        let total = usize::try_from(rows).unwrap_or(usize::MAX);
        let begin = sorted.offset.min(total);
        let end = sorted
            .offset
            .saturating_add(query.limit.unwrap_or(usize::MAX))
            .min(total);
        for position in begin..end {
            let index = if sorted.descending {
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
    mean: f64,
    count: u64,
}

impl AggregateState {
    fn new(function: AggregateFunction) -> Self {
        Self {
            function,
            value: None,
            mean: 0.0,
            count: 0,
        }
    }

    fn update(&mut self, value: Number) -> Result<(), ExecutionError> {
        let next_count = self
            .count
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("aggregate row count overflow".into()))?;
        match self.function {
            AggregateFunction::Sum => {
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
            AggregateFunction::Avg => {
                let numeric = match value {
                    Number::Unsigned(n) => n as f64,
                    Number::Signed(n) => n as f64,
                    Number::Float(n) => n,
                };
                self.mean += (numeric - self.mean) / next_count as f64;
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
        if self.count == 0 {
            return "NULL".into();
        }
        match self.function {
            AggregateFunction::Avg => self.mean.to_string(),
            _ => self
                .value
                .map_or_else(String::new, |value| value.to_string()),
        }
    }
}

const MAX_SORTED_ROWS: usize = 100_000;

struct SortedRow {
    key: Number,
    values: Vec<Number>,
    sequence: usize,
    descending: bool,
}

impl PartialEq for SortedRow {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SortedRow {}

impl PartialOrd for SortedRow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortedRow {
    fn cmp(&self, other: &Self) -> Ordering {
        let ordering = compare_numbers(self.key, other.key);
        let ordering = if self.descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| self.sequence.cmp(&other.sequence))
    }
}

struct SortedRows {
    rows: BinaryHeap<SortedRow>,
    requested: usize,
    offset: usize,
    seen: usize,
    descending: bool,
}

impl SortedRows {
    fn new(limit: Option<usize>, descending: bool) -> Self {
        Self {
            rows: BinaryHeap::new(),
            requested: limit.unwrap_or(usize::MAX),
            offset: 0,
            seen: 0,
            descending,
        }
    }

    fn with_offset(
        limit: Option<usize>,
        offset: usize,
        descending: bool,
    ) -> Result<Self, ExecutionError> {
        let mut sorted = Self::new(limit, descending);
        sorted.requested = limit
            .map(|limit| {
                limit
                    .checked_add(offset)
                    .ok_or_else(|| ExecutionError::Invalid("LIMIT plus OFFSET overflows".into()))
            })
            .transpose()?
            .unwrap_or(usize::MAX);
        sorted.offset = offset;
        Ok(sorted)
    }

    fn push(&mut self, key: Number, values: Vec<Number>) -> Result<(), ExecutionError> {
        if self.requested > MAX_SORTED_ROWS && self.seen >= MAX_SORTED_ROWS {
            return Err(ExecutionError::Invalid(
                "ORDER BY requires LIMIT <= 100000 for large results".into(),
            ));
        }
        let sequence = self.seen;
        self.seen = self
            .seen
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("too many rows to sort".into()))?;
        let row = SortedRow {
            key,
            values,
            sequence,
            descending: self.descending,
        };
        let retained = self.requested.min(MAX_SORTED_ROWS);
        if self.rows.len() < retained {
            self.rows.push(row);
        } else if self.rows.peek().is_some_and(|worst| row < *worst) {
            self.rows.pop();
            self.rows.push(row);
        }
        Ok(())
    }

    fn write<W: Write>(self, output: &mut W) -> Result<(), ExecutionError> {
        for row in self.rows.into_sorted_vec().into_iter().skip(self.offset) {
            for (index, value) in row.values.iter().enumerate() {
                if index != 0 {
                    write!(output, "\t")?;
                }
                write!(output, "{value}")?;
            }
            writeln!(output)?;
        }
        Ok(())
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

// Short IN lists are cheap to scan; long lists benefit from constant-time lookups.
const HASHED_IN_MIN_VALUES: usize = 16;

enum CompiledTest {
    Comparison {
        operator: Operator,
        target: Number,
    },
    In(Vec<Number>),
    NotIn(Vec<Number>),
    NotBetween {
        lower: Number,
        upper: Number,
    },
    InSet {
        targets: HashSet<NumberKey>,
        negated: bool,
    },
}

fn compile_in_test(targets: Vec<Number>, negated: bool) -> CompiledTest {
    if targets.len() < HASHED_IN_MIN_VALUES {
        if negated {
            CompiledTest::NotIn(targets)
        } else {
            CompiledTest::In(targets)
        }
    } else {
        CompiledTest::InSet {
            targets: targets.into_iter().map(NumberKey::from).collect(),
            negated,
        }
    }
}

impl CompiledTest {
    fn matches(&self, value: Number) -> bool {
        match self {
            Self::Comparison { operator, target } => number_matches(value, *target, *operator),
            Self::In(targets) => targets
                .iter()
                .any(|target| number_matches(value, *target, Operator::Eq)),
            Self::NotIn(targets) => targets
                .iter()
                .all(|target| !number_matches(value, *target, Operator::Eq)),
            Self::NotBetween { lower, upper } => {
                !number_matches(value, *lower, Operator::Ge)
                    || !number_matches(value, *upper, Operator::Le)
            }
            Self::InSet { targets, negated } => {
                // Float NaN does not compare equal to any IN-list value.
                let contains = !matches!(value, Number::Float(number) if number.is_nan())
                    && targets.contains(&NumberKey::from(value));
                contains != *negated
            }
        }
    }
}

struct CompiledPredicate {
    index: usize,
    test: CompiledTest,
}

fn compile_part_predicates(
    predicates: &[Predicate],
    scan_columns: &mut Vec<String>,
    mut convert: impl FnMut(&str, &str) -> Result<Number, ExecutionError>,
) -> Result<Vec<CompiledPredicate>, ExecutionError> {
    let mut compiled = Vec::with_capacity(predicates.len());
    for predicate in predicates {
        let column = predicate.column();
        let test = match predicate {
            Predicate::Comparison {
                operator, value, ..
            } => {
                let target = convert(column, value)?;
                if matches!(*operator, Operator::Modulo(_)) && matches!(target, Number::Float(_)) {
                    return Err(ExecutionError::Invalid(
                        "modulo requires integer columns".into(),
                    ));
                }
                CompiledTest::Comparison {
                    operator: *operator,
                    target,
                }
            }
            Predicate::In { values, .. } => compile_in_test(
                values
                    .iter()
                    .map(|value| convert(column, value))
                    .collect::<Result<Vec<_>, _>>()?,
                false,
            ),
            Predicate::NotIn { values, .. } => compile_in_test(
                values
                    .iter()
                    .map(|value| convert(column, value))
                    .collect::<Result<Vec<_>, _>>()?,
                true,
            ),
            Predicate::NotBetween { lower, upper, .. } => CompiledTest::NotBetween {
                lower: convert(column, lower)?,
                upper: convert(column, upper)?,
            },
        };
        let index = if let Some(index) = scan_columns
            .iter()
            .position(|candidate| candidate.as_str() == column)
        {
            index
        } else {
            scan_columns.push(column.to_owned());
            scan_columns.len() - 1
        };
        compiled.push(CompiledPredicate { index, test });
    }
    Ok(compiled)
}

fn batch_matches(batch: &[NumericColumn], row: usize, branches: &[Vec<CompiledPredicate>]) -> bool {
    branches.iter().any(|predicates| {
        predicates.iter().all(|predicate| {
            predicate
                .test
                .matches(number_at(&batch[predicate.index], row))
        })
    })
}

fn compile_part_where(
    query: &Select,
    scan_columns: &mut Vec<String>,
    mut convert: impl FnMut(&str, &str) -> Result<Number, ExecutionError>,
) -> Result<Vec<Vec<CompiledPredicate>>, ExecutionError> {
    let mut result = vec![compile_part_predicates(
        &query.predicates,
        scan_columns,
        |column, value| convert(column, value),
    )?];
    for predicates in &query.or_predicates {
        result.push(compile_part_predicates(
            predicates,
            scan_columns,
            |column, value| convert(column, value),
        )?);
    }
    Ok(result)
}

fn compile_numbers_predicates(
    query: &Select,
) -> Result<Vec<Vec<CompiledPredicate>>, ExecutionError> {
    let mut scan_columns = vec!["number".into()];
    compile_part_where(query, &mut scan_columns, |column, value| {
        if column != "number" {
            return Err(ExecutionError::Invalid(
                "numbers(N) exposes only the number column".into(),
            ));
        }
        parse_numeric("UInt64", value)
    })
}

fn number_matches_all(value: Number, branches: &[Vec<CompiledPredicate>]) -> bool {
    branches.iter().any(|predicates| {
        predicates
            .iter()
            .all(|predicate| predicate.test.matches(value))
    })
}

fn parse_numeric(data_type: &str, text: &str) -> Result<Number, ExecutionError> {
    let value = match data_type {
        "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" => {
            text.parse::<u128>().ok().map(Number::Unsigned)
        }
        "Int8" | "Int16" | "Int32" | "Int64" | "Int128" => {
            text.parse::<i128>().ok().map(Number::Signed)
        }
        "Float32" => text
            .parse::<f32>()
            .ok()
            .map(|value| Number::Float(f64::from(value))),
        "Float64" => text.parse::<f64>().ok().map(Number::Float),
        other => {
            return Err(ExecutionError::Invalid(format!(
                "unsupported WHERE column type: {other}"
            )));
        }
    };
    value.ok_or_else(|| ExecutionError::Invalid(format!("invalid numeric WHERE value: {text}")))
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

fn number_matches(value: Number, target: Number, operator: Operator) -> bool {
    if let Operator::Modulo(modulus) = operator {
        return match (value, target) {
            (Number::Unsigned(value), Number::Unsigned(target)) => {
                value % u128::from(modulus) == target
            }
            (Number::Signed(value), Number::Signed(target)) => {
                value % i128::from(modulus) == target
            }
            _ => false,
        };
    }
    match (value, target) {
        (Number::Unsigned(value), Number::Unsigned(target)) => compare(value, target, operator),
        (Number::Signed(value), Number::Signed(target)) => compare(value, target, operator),
        (Number::Float(value), Number::Float(target)) => compare(value, target, operator),
        _ => false,
    }
}

fn compare<T: PartialOrd>(value: T, target: T, operator: Operator) -> bool {
    match operator {
        Operator::Eq => value == target,
        Operator::Ne => value != target,
        Operator::Lt => value < target,
        Operator::Le => value <= target,
        Operator::Gt => value > target,
        Operator::Ge => value >= target,
        Operator::Modulo(_) => unreachable!("modulo handled before comparison"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_select;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn run(query: &str, part: Option<&Path>) -> Result<String, ExecutionError> {
        let select = parse_select(query).unwrap();
        let mut output = Vec::new();
        execute(&select, part, &mut output)?;
        Ok(String::from_utf8(output).unwrap())
    }

    #[test]
    fn runs_scalar_queries_without_a_part() {
        assert_eq!(run("SELECT 1", None).unwrap(), "1\n1\n");
        assert_eq!(run("SELECT -5 LIMIT 0", None).unwrap(), "-5\n");
    }

    #[test]
    fn clickhouse_count_empty_parentheses_matches_count_star() {
        assert_eq!(
            run("SELECT count() FROM numbers(17) WHERE number >= 12", None).unwrap(),
            run("SELECT count(*) FROM numbers(17) WHERE number >= 12", None).unwrap()
        );
        assert_eq!(
            run(
                "SELECT number, count() FROM numbers(3) GROUP BY number",
                None
            )
            .unwrap(),
            run(
                "SELECT number, count(*) FROM numbers(3) GROUP BY number",
                None
            )
            .unwrap()
        );
    }

    #[test]
    fn runs_generated_numbers_with_projection_filter_count_and_limit() {
        assert_eq!(
            run("SELECT * FROM numbers(4)", None).unwrap(),
            "number\n0\n1\n2\n3\n"
        );
        assert_eq!(
            run(
                "SELECT number, number FROM numbers(5) WHERE number >= 3 LIMIT 1",
                None,
            )
            .unwrap(),
            "number\tnumber\n3\t3\n"
        );
        assert_eq!(
            run("SELECT count(*) FROM numbers(5)", None).unwrap(),
            "count\n5\n"
        );
        assert_eq!(
            run("SELECT count(*) FROM numbers(5) WHERE number < 3", None).unwrap(),
            "count\n3\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(0)", None).unwrap(),
            "number\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(4) LIMIT 0", None).unwrap(),
            "number\n"
        );
        assert!(run("SELECT missing FROM numbers(0)", None).is_err());
        assert!(run("SELECT number FROM numbers(0) WHERE missing = 1", None).is_err());
        assert!(run("SELECT number FROM numbers(0) WHERE number = -1", None).is_err());
        assert!(run("SELECT number FROM numbers(2)", Some(Path::new("."))).is_err());
    }

    #[test]
    fn queries_numbers_with_start_and_count() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(7, 4) WHERE number >= 8 LIMIT 2",
                None
            )
            .unwrap(),
            "number\n8\n9\n"
        );
        assert_eq!(
            run("SELECT count(*) FROM numbers(7, 4)", None).unwrap(),
            "count\n4\n"
        );
        assert_eq!(
            run("SELECT sum(number) FROM numbers(7, 4)", None).unwrap(),
            "sum(number)\n34\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(7, 0)", None).unwrap(),
            "number\n"
        );
    }

    #[test]
    fn stepped_numbers_respect_clickhouse_range_semantics() {
        assert_eq!(
            run("SELECT number FROM numbers(3, 10, 4)", None).unwrap(),
            "number\n3\n7\n11\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(3, 10, 4) LIMIT 1 OFFSET 2",
                None
            )
            .unwrap(),
            "number\n11\n"
        );
        assert_eq!(
            run("SELECT count(*) FROM numbers(3, 10, 4)", None).unwrap(),
            "count\n3\n"
        );
        assert_eq!(
            run("SELECT sum(number) FROM numbers(3, 10, 4)", None).unwrap(),
            "sum(number)\n21\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(5, 0, 2)", None).unwrap(),
            "number\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(5, 3, 100)", None).unwrap(),
            "number\n5\n"
        );
    }

    #[test]
    fn filters_with_modulo_for_generated_numbers() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(12) WHERE number % 3 = 1 ORDER BY number DESC LIMIT 2",
                None,
            )
            .unwrap(),
            "number\n10\n7\n"
        );
        assert_eq!(
            run(
                "SELECT count(*) FROM numbers(10) WHERE number % 2 = 0",
                None
            )
            .unwrap(),
            "count\n5\n"
        );
    }

    #[test]
    fn evaluates_not_in_with_filters_and_aggregates() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number NOT IN (1, 3, 5) LIMIT 4",
                None,
            )
            .unwrap(),
            "number\n0\n2\n4\n6\n"
        );
        assert_eq!(
            run(
                "SELECT count(*) FROM numbers(10) WHERE number NOT IN (1, 3, 5)",
                None,
            )
            .unwrap(),
            "count\n7\n"
        );
    }

    #[test]
    fn evaluates_or_across_generated_rows() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number = 1 OR number >= 4 AND number < 6",
                None,
            )
            .unwrap(),
            "number\n1\n4\n5\n"
        );
        assert_eq!(
            run(
                "SELECT count(*) FROM numbers(10) WHERE number = 0 OR number = 9",
                None,
            )
            .unwrap(),
            "count\n2\n"
        );
    }

    #[test]
    fn counts_exact_distinct_numeric_values_after_filters() {
        assert_eq!(
            run("SELECT count(DISTINCT number) FROM numbers(10)", None).unwrap(),
            "countDistinct(number)\n10\n"
        );
        assert_eq!(
            run(
                "SELECT count(DISTINCT number) FROM numbers(10) WHERE number < 3 OR number >= 8",
                None
            )
            .unwrap(),
            "countDistinct(number)\n5\n"
        );
        assert_eq!(
            run("SELECT count(DISTINCT number) FROM numbers(0)", None).unwrap(),
            "countDistinct(number)\n0\n"
        );
        assert!(run("SELECT count(DISTINCT missing) FROM numbers(3)", None).is_err());
    }

    #[test]
    fn generated_count_distinct_skips_unfiltered_scans_and_respects_steps() {
        assert_eq!(
            run("SELECT count(DISTINCT number) FROM numbers(1000000)", None).unwrap(),
            "countDistinct(number)\n1000000\n"
        );
        assert_eq!(
            run("SELECT count(DISTINCT number) FROM numbers(3, 10, 4)", None).unwrap(),
            "countDistinct(number)\n3\n"
        );
        assert_eq!(
            run(
                "SELECT count(DISTINCT number) FROM numbers(3, 10, 4) WHERE number >= 7",
                None
            )
            .unwrap(),
            "countDistinct(number)\n2\n"
        );
        assert_eq!(
            run(
                "SELECT count(DISTINCT number) FROM numbers(3, 10, 4) WHERE number = 11",
                None
            )
            .unwrap(),
            "countDistinct(number)\n1\n"
        );
        assert_eq!(
            run("SELECT count(DISTINCT number) FROM numbers(0)", None).unwrap(),
            "countDistinct(number)\n0\n"
        );
        assert_eq!(
            run(
                "SELECT count(DISTINCT number) FROM numbers(4) LIMIT 0",
                None
            )
            .unwrap(),
            "countDistinct(number)\n"
        );
    }

    #[test]
    fn aggregates_generated_numbers_and_empty_inputs() {
        assert_eq!(
            run("SELECT sum(number) FROM numbers(10)", None).unwrap(),
            "sum(number)\n45\n"
        );
        assert_eq!(
            run("SELECT AVG(number) FROM numbers(10)", None).unwrap(),
            "avg(number)\n4.5\n"
        );
        assert_eq!(
            run(
                "SELECT sum(number) FROM numbers(10) WHERE number >= 7",
                None,
            )
            .unwrap(),
            "sum(number)\n24\n"
        );
        assert_eq!(
            run("SELECT min(number) FROM numbers(10)", None).unwrap(),
            "min(number)\n0\n"
        );
        assert_eq!(
            run("SELECT max(number) FROM numbers(10)", None).unwrap(),
            "max(number)\n9\n"
        );
        assert_eq!(
            run("SELECT avg(number) FROM numbers(0)", None).unwrap(),
            "avg(number)\nNULL\n"
        );
        assert_eq!(
            run("SELECT sum(number) FROM numbers(5) WHERE number > 10", None).unwrap(),
            "sum(number)\nNULL\n"
        );
        assert_eq!(
            run("SELECT sum(number) FROM numbers(5) LIMIT 0", None).unwrap(),
            "sum(number)\n"
        );
        assert!(run("SELECT sum(missing) FROM numbers(0)", None).is_err());
    }

    #[test]
    fn exact_unfiltered_generated_aggregates_match_scanned_values() {
        for (start, length, step) in [
            (0, 0, 1),
            (0, 17, 1),
            (3, 10, 4),
            (1000, 1, 10),
            (u64::MAX - 20, 20, 6),
        ] {
            let rows = length / step + u64::from(length % step != 0);
            for function in [
                AggregateFunction::Sum,
                AggregateFunction::Min,
                AggregateFunction::Max,
            ] {
                let mut state = AggregateState::new(function);
                for index in 0..rows {
                    state
                        .update(Number::Unsigned(u128::from(start + index * step)))
                        .unwrap();
                }
                let sql = format!(
                    "SELECT {}(number) FROM numbers({start}, {length}, {step})",
                    function.name()
                );
                assert_eq!(
                    run(&sql, None).unwrap(),
                    format!("{}(number)\n{}\n", function.name(), state.render()),
                    "{sql}"
                );
            }
        }
        assert_eq!(
            run("SELECT sum(number) FROM numbers(1000000)", None).unwrap(),
            "sum(number)\n499999500000\n"
        );
        assert_eq!(
            run(
                "SELECT sum(number) FROM numbers(10) WHERE number >= 8",
                None
            )
            .unwrap(),
            "sum(number)\n17\n"
        );
    }

    #[test]
    fn aggregate_state_preserves_precision_and_rejects_overflow() {
        let mut sum = AggregateState::new(AggregateFunction::Sum);
        sum.update(Number::Unsigned(u128::MAX)).unwrap();
        assert!(sum.update(Number::Unsigned(1)).is_err());

        let mut min = AggregateState::new(AggregateFunction::Min);
        min.update(Number::Signed(i128::MAX)).unwrap();
        min.update(Number::Signed(i128::MIN)).unwrap();
        assert_eq!(min.render(), i128::MIN.to_string());
    }

    #[test]
    fn sorts_numbers_with_top_k_and_filtered_rows() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) ORDER BY number DESC LIMIT 3",
                None,
            )
            .unwrap(),
            "number\n9\n8\n7\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(8) WHERE number >= 4 ORDER BY number ASC LIMIT 2",
                None,
            )
            .unwrap(),
            "number\n4\n5\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(3) ORDER BY number DESC", None).unwrap(),
            "number\n2\n1\n0\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(3) ORDER BY number LIMIT 0",
                None,
            )
            .unwrap(),
            "number\n"
        );
        assert!(run("SELECT number FROM numbers(0) ORDER BY missing", None).is_err());
    }

    #[test]
    fn orders_unfiltered_generated_ranges_without_scanning() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(1000000) ORDER BY number DESC LIMIT 2 OFFSET 1000",
                None
            )
            .unwrap(),
            "number\n998999\n998998\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(1000000) ORDER BY number LIMIT 1 OFFSET 99999",
                None
            )
            .unwrap(),
            "number\n99999\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(20, 13, 4) ORDER BY number DESC LIMIT 2 OFFSET 1",
                None
            )
            .unwrap(),
            "number\n28\n24\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(3) ORDER BY number DESC LIMIT 2 OFFSET 5",
                None
            )
            .unwrap(),
            "number\n"
        );
        assert!(run("SELECT number FROM numbers(100001) ORDER BY number", None).is_err());
        assert!(
            run(
                "SELECT number FROM numbers(100001) ORDER BY number LIMIT 100001",
                None
            )
            .is_err()
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(100001) ORDER BY number LIMIT 0",
                None
            )
            .unwrap(),
            "number\n"
        );
    }

    #[test]
    fn not_between_filters_generated_numbers_and_combines_with_or() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number NOT BETWEEN 3 AND 6",
                None
            )
            .unwrap(),
            "number\n0\n1\n2\n7\n8\n9\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number NOT BETWEEN 3 AND 6 AND number < 9 OR number = 9",
                None
            )
            .unwrap(),
            "number\n0\n1\n2\n7\n8\n9\n"
        );
        assert_eq!(
            run(
                "SELECT count(*) FROM numbers(10) WHERE number NOT BETWEEN 0 AND 9",
                None
            )
            .unwrap(),
            "count\n0\n"
        );
    }

    #[test]
    fn between_supports_inclusive_ranges_and_chained_filters() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number BETWEEN 3 AND 7",
                None
            )
            .unwrap(),
            "number\n3\n4\n5\n6\n7\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number BETWEEN 3 AND 8 AND number != 5 ORDER BY number DESC LIMIT 3 OFFSET 1",
                None,
            )
            .unwrap(),
            "number\n7\n6\n4\n"
        );
        assert_eq!(
            run(
                "SELECT count(*) FROM numbers(10) WHERE number BETWEEN 3 AND 7",
                None
            )
            .unwrap(),
            "count\n5\n"
        );
        assert_eq!(
            run(
                "SELECT sum(number) FROM numbers(10) WHERE number BETWEEN 3 AND 7",
                None
            )
            .unwrap(),
            "sum(number)\n25\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number BETWEEN 7 AND 3",
                None
            )
            .unwrap(),
            "number\n"
        );
        assert!(
            run(
                "SELECT number FROM numbers(0) WHERE number BETWEEN -1 AND 3",
                None
            )
            .is_err()
        );
    }

    #[test]
    fn multiple_where_conditions_filter_numbers_counts_and_aggregates() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number >= 3 AND number < 7",
                None,
            )
            .unwrap(),
            "number\n3\n4\n5\n6\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number > 3 AND number <= 8 ORDER BY number DESC LIMIT 2",
                None,
            )
            .unwrap(),
            "number\n8\n7\n"
        );
        assert_eq!(
            run(
                "SELECT count(*) FROM numbers(10) WHERE number >= 2 AND number < 5",
                None,
            )
            .unwrap(),
            "count\n3\n"
        );
        assert_eq!(
            run(
                "SELECT sum(number) FROM numbers(10) WHERE number >= 2 AND number < 5",
                None,
            )
            .unwrap(),
            "sum(number)\n9\n"
        );
        assert!(
            run(
                "SELECT number FROM numbers(0) WHERE number > 2 AND missing < 5",
                None
            )
            .is_err()
        );
    }

    #[test]
    fn groups_numeric_rows_after_filtering_before_pagination() {
        assert_eq!(
            run(
                "SELECT number, COUNT(*) FROM numbers(5) GROUP BY number",
                None,
            )
            .unwrap(),
            "number\tcount\n0\t1\n1\t1\n2\t1\n3\t1\n4\t1\n"
        );
        assert_eq!(
            run(
                "SELECT number, COUNT(*) FROM numbers(10) WHERE number BETWEEN 2 AND 6 GROUP BY number ORDER BY number DESC LIMIT 2 OFFSET 1",
                None,
            )
            .unwrap(),
            "number\tcount\n5\t1\n4\t1\n"
        );
        assert_eq!(
            run(
                "SELECT number, COUNT(*) FROM numbers(0) GROUP BY number",
                None
            )
            .unwrap(),
            "number\tcount\n"
        );
        assert_eq!(
            run(
                "SELECT number, COUNT(*) FROM numbers(3) GROUP BY number LIMIT 0",
                None
            )
            .unwrap(),
            "number\tcount\n"
        );
        assert!(run("SELECT bad, COUNT(*) FROM numbers(0) GROUP BY bad", None).is_err());
    }

    #[test]
    fn grouped_having_filters_after_counting_before_limit() {
        let query = crate::parse_select(
            "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING count(*) >= 2 ORDER BY number LIMIT 1",
        )
        .unwrap();
        let mut groups = GroupedCounts::default();
        for value in [3, 3, 3, 1, 1, 5] {
            groups.push(Number::Unsigned(value)).unwrap();
        }
        let mut result = Vec::new();
        groups.write(&query, "number", &mut result).unwrap();
        assert_eq!(String::from_utf8(result).unwrap(), "number\tcount\n1\t2\n");
        assert_eq!(
            run(
                "SELECT number, count(*) FROM numbers(3) GROUP BY number HAVING count(*) > 1",
                None
            )
            .unwrap(),
            "number\tcount\n"
        );
    }

    #[test]
    fn grouped_counts_support_ranges_and_or_filters() {
        assert_eq!(
            run(
                "SELECT number, COUNT(*) FROM numbers(8, 4) WHERE number = 8 OR number = 11 GROUP BY number ORDER BY number DESC",
                None
            )
            .unwrap(),
            "number\tcount\n11\t1\n8\t1\n"
        );
    }

    #[test]
    fn grouped_count_order_sorts_by_frequency_then_key() {
        let query = crate::parse_select(
            "SELECT number, count(*) FROM numbers(5) GROUP BY number ORDER BY count() DESC LIMIT 2",
        )
        .unwrap();
        let mut groups = GroupedCounts::default();
        for value in [3, 3, 1, 1, 1, 2] {
            groups.push(Number::Unsigned(value)).unwrap();
        }
        let mut result = Vec::new();
        groups.write(&query, "number", &mut result).unwrap();
        assert_eq!(
            String::from_utf8(result).unwrap(),
            "number\tcount\n1\t3\n3\t2\n"
        );
    }

    #[test]
    fn grouped_counts_merge_equal_values_without_precision_loss() {
        let mut groups = GroupedCounts::default();
        groups.push(Number::Unsigned(u128::MAX)).unwrap();
        groups.push(Number::Unsigned(u128::MAX)).unwrap();
        groups.push(Number::Unsigned(u128::MAX - 1)).unwrap();
        groups.push(Number::Float(0.0)).unwrap();
        groups.push(Number::Float(-0.0)).unwrap();
        assert_eq!(groups.rows.len(), 3);
        assert_eq!(groups.rows[0].1, 2);
        assert_eq!(groups.rows[1].1, 1);
        assert_eq!(groups.rows[2].1, 2);
    }

    #[test]
    fn distinct_keys_preserve_numeric_identity() {
        let mut seen = HashSet::new();
        assert!(remember_distinct(&mut seen, &[Number::Unsigned(u128::MAX)]).unwrap());
        assert!(!remember_distinct(&mut seen, &[Number::Unsigned(u128::MAX)]).unwrap());
        assert!(remember_distinct(&mut seen, &[Number::Unsigned(u128::MAX - 1)]).unwrap());
        assert!(remember_distinct(&mut seen, &[Number::Float(0.0)]).unwrap());
        assert!(!remember_distinct(&mut seen, &[Number::Float(-0.0)]).unwrap());
        assert!(remember_distinct(&mut seen, &[Number::Float(f64::NAN)]).unwrap());
        assert!(!remember_distinct(&mut seen, &[Number::Float(-f64::NAN)]).unwrap());
        assert!(remember_distinct(&mut seen, &[Number::Signed(-1)]).unwrap());
    }

    #[test]
    fn distinct_limit_rejects_new_rows_but_accepts_existing_rows() {
        let mut seen = HashSet::new();
        for index in 0..MAX_DISTINCT_ROWS {
            assert!(remember_distinct(&mut seen, &[Number::Unsigned(index as u128)]).unwrap());
        }
        assert!(!remember_distinct(&mut seen, &[Number::Unsigned(0)]).unwrap());
        assert!(matches!(
            remember_distinct(&mut seen, &[Number::Unsigned(MAX_DISTINCT_ROWS as u128)]),
            Err(ExecutionError::Invalid(_))
        ));
    }

    #[test]
    fn distinct_on_generated_numbers_preserves_unique_rows() {
        assert_eq!(
            run("SELECT DISTINCT number FROM numbers(3)", None).unwrap(),
            "number\n0\n1\n2\n"
        );
        assert_eq!(
            run(
                "SELECT DISTINCT number, number FROM numbers(3) LIMIT 2 OFFSET 1",
                None
            )
            .unwrap(),
            "number\tnumber\n1\t1\n2\t2\n"
        );
        assert_eq!(
            run("SELECT DISTINCT * FROM numbers(0)", None).unwrap(),
            "number\n"
        );
    }

    #[test]
    fn in_lists_filter_numbers_aggregates_and_empty_inputs() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number IN (2, 4, 7) ORDER BY number DESC LIMIT 2 OFFSET 1",
                None,
            )
            .unwrap(),
            "number\n4\n2\n"
        );
        assert_eq!(
            run(
                "SELECT count(*) FROM numbers(10) WHERE number IN (1, 4, 8) AND number > 2",
                None,
            )
            .unwrap(),
            "count\n2\n"
        );
        assert_eq!(
            run(
                "SELECT sum(number) FROM numbers(10) WHERE number IN (3, 5, 7)",
                None
            )
            .unwrap(),
            "sum(number)\n15\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(0) WHERE number IN (1)", None).unwrap(),
            "number\n"
        );
        assert!(run("SELECT number FROM numbers(0) WHERE number IN (-1)", None).is_err());
        assert!(run("SELECT number FROM numbers(0) WHERE missing IN (1)", None).is_err());
    }

    #[test]
    fn large_in_and_not_in_lists_use_hash_lookups_without_changing_results() {
        let values = (0..32).map(|number| number.to_string()).collect::<Vec<_>>();
        let in_query = format!(
            "SELECT count(*) FROM numbers(1000000) WHERE number IN ({})",
            values.join(", ")
        );
        assert_eq!(run(&in_query, None).unwrap(), "count\n32\n");
        let not_in_query = format!(
            "SELECT count(*) FROM numbers(1000000) WHERE number NOT IN ({})",
            values.join(", ")
        );
        assert_eq!(run(&not_in_query, None).unwrap(), "count\n999968\n");

        let float_values = (0..16)
            .map(|number| Number::Float(number as f64))
            .collect::<Vec<_>>();
        let positive = compile_in_test(float_values.clone(), false);
        let negative = compile_in_test(float_values, true);
        assert!(matches!(&positive, CompiledTest::InSet { .. }));
        assert!(positive.matches(Number::Float(-0.0)));
        assert!(positive.matches(Number::Float(15.0)));
        assert!(!positive.matches(Number::Float(16.0)));
        assert!(!positive.matches(Number::Float(f64::NAN)));
        assert!(!positive.matches(Number::Unsigned(1)));
        assert!(!negative.matches(Number::Float(-0.0)));
        assert!(negative.matches(Number::Float(16.0)));
        assert!(negative.matches(Number::Float(f64::NAN)));
        assert!(negative.matches(Number::Unsigned(1)));

        let mut large_values = (0..16)
            .map(|number| Number::Unsigned(number as u128))
            .collect::<Vec<_>>();
        large_values.push(Number::Unsigned(u128::MAX));
        let test = compile_in_test(large_values, false);
        assert!(test.matches(Number::Unsigned(u128::MAX)));
        assert!(!test.matches(Number::Unsigned(u128::MAX - 1)));
    }

    #[test]
    fn pagination_applies_after_filtering_and_sorting() {
        assert_eq!(
            run("SELECT number FROM numbers(10) LIMIT 3 OFFSET 4", None).unwrap(),
            "number\n4\n5\n6\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) ORDER BY number DESC LIMIT 3 OFFSET 2",
                None,
            )
            .unwrap(),
            "number\n7\n6\n5\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number >= 5 LIMIT 2 OFFSET 1",
                None,
            )
            .unwrap(),
            "number\n6\n7\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(2) LIMIT 2 OFFSET 100", None).unwrap(),
            "number\n"
        );
        assert_eq!(
            run("SELECT count(*) FROM numbers(10) LIMIT 1 OFFSET 1", None).unwrap(),
            "count\n"
        );
        assert_eq!(
            run("SELECT sum(number) FROM numbers(10) LIMIT 1 OFFSET 1", None).unwrap(),
            "sum(number)\n"
        );
        assert_eq!(run("SELECT 1 LIMIT 1 OFFSET 1", None).unwrap(), "1\n");
        assert!(SortedRows::with_offset(Some(usize::MAX), 1, false).is_err());
    }

    #[test]
    fn unfiltered_number_offsets_skip_directly_to_the_first_result() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(1000000) LIMIT 1 OFFSET 999999",
                None
            )
            .unwrap(),
            "number\n999999\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(7, 4) LIMIT 2 OFFSET 2", None).unwrap(),
            "number\n9\n10\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(7, 4) LIMIT 2 OFFSET 1000", None).unwrap(),
            "number\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(18446744073709551613, 2) LIMIT 1 OFFSET 1",
                None
            )
            .unwrap(),
            "number\n18446744073709551614\n"
        );
        // Filtered, disjunctive, and sorted queries apply OFFSET to matched rows.
        assert_eq!(
            run(
                "SELECT number FROM numbers(7, 4) WHERE number >= 8 LIMIT 2 OFFSET 1",
                None
            )
            .unwrap(),
            "number\n9\n10\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(10) WHERE number < 3 OR number > 6 LIMIT 2 OFFSET 1",
                None
            )
            .unwrap(),
            "number\n1\n2\n"
        );
        // The OR compiler always builds one branch, even for an empty WHERE.
        assert_eq!(
            run("SELECT count(*) FROM numbers(1000000)", None).unwrap(),
            "count\n1000000\n"
        );
    }

    #[test]
    fn sorting_keeps_stable_ties_and_large_integer_precision() {
        let mut sorted = SortedRows::new(Some(3), true);
        for (key, value) in [(4, 0), (2, 1), (4, 2), (5, 3)] {
            sorted
                .push(Number::Signed(key), vec![Number::Unsigned(value)])
                .unwrap();
        }
        let mut output = Vec::new();
        sorted.write(&mut output).unwrap();
        assert_eq!(output, b"3\n0\n2\n");

        let mut sorted = SortedRows::new(Some(1), false);
        for key in [u128::MAX, 0, u128::MAX - 1] {
            sorted
                .push(Number::Unsigned(key), vec![Number::Unsigned(key)])
                .unwrap();
        }
        let mut output = Vec::new();
        sorted.write(&mut output).unwrap();
        assert_eq!(output, b"0\n");
    }

    #[test]
    fn count_without_where_uses_part_metadata() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-count-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("count.txt"), "42\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n\u{0060}city\u{0060} String\n",
        )
        .unwrap();
        assert_eq!(
            run("SELECT count(*) FROM part", Some(&dir)).unwrap(),
            "count\n42\n"
        );
        assert_eq!(
            run("SELECT count(*) FROM part LIMIT 0", Some(&dir)).unwrap(),
            "count\n"
        );
        assert!(run("SELECT city FROM part", Some(&dir)).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn checks_predicates_on_empty_parts_and_before_limit_zero() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-empty-where-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("count.txt"), "0\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` UInt64\n",
        )
        .unwrap();

        for statement in [
            "SELECT id FROM part WHERE id = -1",
            "SELECT id FROM part WHERE id = -1 LIMIT 0",
            "SELECT count(*) FROM part WHERE id = -1",
        ] {
            let select = parse_select(statement).unwrap();
            let mut output = Vec::new();
            assert!(
                matches!(
                    execute(&select, Some(&dir), &mut output),
                    Err(ExecutionError::Invalid(_))
                ),
                "{statement}"
            );
            assert!(output.is_empty());
        }

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_unknown_sources_and_missing_part_directory() {
        assert!(matches!(
            run("SELECT id FROM other", Some(Path::new("."))),
            Err(ExecutionError::Invalid(_))
        ));
        assert!(matches!(
            run("SELECT id FROM part", None),
            Err(ExecutionError::Invalid(_))
        ));
    }

    #[test]
    fn numeric_comparisons_do_not_lose_large_integer_precision() {
        assert!(number_matches(
            Number::Unsigned(u128::MAX),
            Number::Unsigned(u128::MAX - 1),
            Operator::Gt,
        ));
        assert!(number_matches(
            Number::Signed(i128::MIN),
            Number::Signed(0),
            Operator::Lt,
        ));
        assert!(!number_matches(
            Number::Float(f64::NAN),
            Number::Float(1.0),
            Operator::Eq,
        ));
        assert!(number_matches(
            number_at(&NumericColumn::UInt128(vec![u128::MAX]), 0),
            parse_numeric("UInt128", &u128::MAX.to_string()).unwrap(),
            Operator::Eq,
        ));
        assert!(number_matches(
            number_at(&NumericColumn::Int128(vec![i128::MIN]), 0),
            parse_numeric("Int128", &i128::MIN.to_string()).unwrap(),
            Operator::Eq,
        ));
    }
}
