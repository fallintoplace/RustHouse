//! Bounded typed SELECT path for String, UUID and Nullable wide-part columns.
//! Keep the numeric-only executor unchanged for its optimized fast paths.
use super::{ExecutionError, Number, NumberKey, compare_numbers, number_at};
use crate::{Operator, Predicate, Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_BUFFERED_ROWS: usize = 100_000;

#[derive(Clone)]
pub(super) enum Cell {
    Numeric(Number),
    Text(Vec<u8>),
    Uuid(String),
    Null,
}

#[derive(PartialEq, Eq, Hash)]
pub(super) enum Key {
    Numeric(NumberKey),
    Text(Vec<u8>),
    Uuid(String),
    Null,
}

impl Cell {
    pub(super) fn key(&self) -> Key {
        match self {
            Self::Numeric(value) => Key::Numeric(NumberKey::from(*value)),
            Self::Text(bytes) => Key::Text(bytes.clone()),
            Self::Uuid(value) => Key::Uuid(value.clone()),
            Self::Null => Key::Null,
        }
    }
}

pub(super) fn cell(column: &TypedColumn, row: usize) -> Cell {
    match column {
        TypedColumn::Numeric(values) => Cell::Numeric(number_at(values, row)),
        TypedColumn::Text(values) => Cell::Text(values.get(row).unwrap().to_owned()),
        TypedColumn::Uuid(values) => Cell::Uuid(values[row].to_string()),
        TypedColumn::NullableNumeric { values, nulls } => {
            if nulls[row] != 0 {
                Cell::Null
            } else {
                Cell::Numeric(number_at(values, row))
            }
        }
        TypedColumn::NullableText { values, nulls } => {
            if nulls[row] != 0 {
                Cell::Null
            } else {
                Cell::Text(values.get(row).unwrap().to_owned())
            }
        }
    }
}

fn is_typed(data_type: &str) -> bool {
    data_type == "String" || data_type == "UUID" || data_type.starts_with("Nullable(")
}

pub(super) fn requires_typed_scan(query: &Select, path: &Path) -> Result<bool, ExecutionError> {
    let metadata = inspect_query_source(path)?;
    let mut names = Vec::new();
    match &query.projection {
        Projection::All => names.extend(metadata.columns.iter().map(|c| c.name.as_str())),
        Projection::Columns(values) => names.extend(values.iter().map(String::as_str)),
        Projection::Aggregate { column, .. }
        | Projection::CountColumn { column }
        | Projection::CountDistinct { column }
        | Projection::GroupedCount { column } => names.push(column),
        _ => {}
    }
    for pred in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if matches!(pred, Predicate::IsNull { .. } | Predicate::Like { .. }) {
            return Ok(true);
        }
        names.push(pred.column());
    }
    for order in query.order_terms() {
        names.push(query.order_source_column(&order.column));
    }
    Ok(metadata
        .columns
        .iter()
        .any(|column| is_typed(&column.data_type) && names.contains(&column.name.as_str())))
}

pub(super) fn base_type(data_type: &str) -> &str {
    data_type
        .strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(data_type)
}

fn parse_literal(data_type: &str, literal: &str) -> Result<Cell, ExecutionError> {
    let kind = base_type(data_type);
    match kind {
        "String" => Ok(Cell::Text(literal.as_bytes().to_vec())),
        "UUID" => {
            let valid = literal.len() == 36
                && literal.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_hexdigit()
                    }
                });
            if !valid {
                return Err(ExecutionError::Invalid("invalid UUID literal".into()));
            }
            Ok(Cell::Uuid(literal.to_ascii_lowercase()))
        }
        other => {
            let numeric_kind = match other {
                "Date" => "UInt16",
                "Date32" => "Int32",
                "DateTime" => "UInt32",
                other if other.starts_with("DateTime('") => "UInt32",
                other if other.starts_with("DateTime64(") => "Int64",
                other => other,
            };
            super::predicates::parse_numeric(numeric_kind, literal).map(Cell::Numeric)
        }
    }
}

fn parse_literals(data_type: &str, values: &[String]) -> Result<Vec<Cell>, ExecutionError> {
    values
        .iter()
        .map(|value| parse_literal(data_type, value))
        .collect()
}

#[derive(Clone, Copy)]
enum LikeToken {
    Any,
    One,
    Byte(u8),
}

fn compile_like(pattern: &str) -> Result<Vec<LikeToken>, ExecutionError> {
    let mut tokens = Vec::new();
    let mut bytes = pattern.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'%' => {
                if !matches!(tokens.last(), Some(LikeToken::Any)) {
                    tokens.push(LikeToken::Any);
                }
            }
            b'_' => tokens.push(LikeToken::One),
            b'\\' => {
                let escaped = bytes.next().ok_or_else(|| {
                    ExecutionError::Invalid("LIKE pattern ends with an escape".into())
                })?;
                if matches!(escaped, b'%' | b'_' | b'\\') {
                    tokens.push(LikeToken::Byte(escaped));
                } else {
                    tokens.push(LikeToken::Byte(b'\\'));
                    tokens.push(LikeToken::Byte(escaped));
                }
            }
            literal => tokens.push(LikeToken::Byte(literal)),
        }
    }
    Ok(tokens)
}

fn next_utf8_boundary(value: &[u8], index: usize) -> Option<usize> {
    let width = match *value.get(index)? {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let end = index.checked_add(width)?;
    std::str::from_utf8(value.get(index..end)?).ok()?;
    Some(end)
}

fn like_matches(value: &[u8], pattern: &[LikeToken], case_insensitive: bool) -> bool {
    let (mut row, mut pat) = (0, 0);
    let mut last_any = None;
    let mut restart = 0;
    while row < value.len() {
        match pattern.get(pat) {
            Some(LikeToken::Byte(byte))
                if *byte == value[row]
                    || (case_insensitive && value[row].eq_ignore_ascii_case(byte)) =>
            {
                row += 1;
                pat += 1;
            }
            Some(LikeToken::One) => {
                let Some(next) = next_utf8_boundary(value, row) else {
                    return false;
                };
                row = next;
                pat += 1;
            }
            Some(LikeToken::Any) => {
                last_any = Some(pat);
                pat += 1;
                restart = row;
            }
            _ => {
                let Some(any) = last_any else {
                    return false;
                };
                let Some(next) = next_utf8_boundary(value, restart) else {
                    return false;
                };
                restart = next;
                row = restart;
                pat = any + 1;
            }
        }
    }
    while matches!(pattern.get(pat), Some(LikeToken::Any)) {
        pat += 1;
    }
    pat == pattern.len()
}

enum Test {
    Compare(Operator, Cell),
    In(Vec<Cell>, bool),
    NotBetween(Cell, Cell),
    IsNull(bool),
    Like {
        pattern: Vec<LikeToken>,
        case_insensitive: bool,
        negated: bool,
    },
}

pub(super) struct Condition {
    index: usize,
    test: Test,
}

pub(super) fn compile(
    predicates: &[Predicate],
    indices: &HashMap<String, usize>,
    types: &HashMap<String, String>,
) -> Result<Vec<Condition>, ExecutionError> {
    predicates
        .iter()
        .map(|pred| {
            let name = pred.column();
            let index = *indices
                .get(name)
                .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
            let kind = types
                .get(name)
                .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
            let test = match pred {
                Predicate::Comparison {
                    operator, value, ..
                } => Test::Compare(*operator, parse_literal(kind, value)?),
                Predicate::In { values, .. } => Test::In(parse_literals(kind, values)?, false),
                Predicate::NotIn { values, .. } => Test::In(parse_literals(kind, values)?, true),
                Predicate::NotBetween { lower, upper, .. } => {
                    Test::NotBetween(parse_literal(kind, lower)?, parse_literal(kind, upper)?)
                }
                Predicate::IsNull { negated, .. } => Test::IsNull(*negated),
                Predicate::Like {
                    pattern,
                    case_insensitive,
                    negated,
                    ..
                } => {
                    if base_type(kind) != "String" {
                        return Err(ExecutionError::Invalid(
                            "LIKE requires a String column".into(),
                        ));
                    }
                    Test::Like {
                        pattern: compile_like(pattern)?,
                        case_insensitive: *case_insensitive,
                        negated: *negated,
                    }
                }
            };
            Ok(Condition { index, test })
        })
        .collect()
}

pub(super) fn cmp(left: &Cell, right: &Cell) -> Ordering {
    match (left, right) {
        (Cell::Numeric(left), Cell::Numeric(right)) => compare_numbers(*left, *right),
        (Cell::Text(left), Cell::Text(right)) => left.cmp(right),
        (Cell::Uuid(left), Cell::Uuid(right)) => left.cmp(right),
        (Cell::Null, Cell::Null) => Ordering::Equal,
        (Cell::Null, _) => Ordering::Less,
        (_, Cell::Null) => Ordering::Greater,
        (Cell::Numeric(_), _) => Ordering::Less,
        (_, Cell::Numeric(_)) => Ordering::Greater,
        (Cell::Text(_), _) => Ordering::Less,
        (_, Cell::Text(_)) => Ordering::Greater,
    }
}

fn compare_order_cells(
    left: &Cell,
    right: &Cell,
    descending: bool,
    nulls_first: Option<bool>,
) -> Ordering {
    match (left, right, nulls_first) {
        (Cell::Null, Cell::Null, _) => Ordering::Equal,
        (Cell::Null, _, Some(true)) | (_, Cell::Null, Some(false)) => Ordering::Less,
        (Cell::Null, _, Some(false)) | (_, Cell::Null, Some(true)) => Ordering::Greater,
        _ => {
            let comparison = cmp(left, right);
            if descending {
                comparison.reverse()
            } else {
                comparison
            }
        }
    }
}

fn matches_comparison(value: &Cell, target: &Cell, operator: Operator) -> bool {
    if matches!(value, Cell::Null) || matches!(target, Cell::Null) {
        return false;
    }
    if let (Cell::Numeric(left), Cell::Numeric(right)) = (value, target) {
        return super::predicates::number_matches(*left, *right, operator);
    }
    if !matches!(
        operator,
        Operator::Eq | Operator::Ne | Operator::Lt | Operator::Le | Operator::Gt | Operator::Ge
    ) {
        return false;
    }
    if !matches!(
        (value, target),
        (Cell::Text(_), Cell::Text(_)) | (Cell::Uuid(_), Cell::Uuid(_))
    ) {
        return false;
    }
    let ordering = cmp(value, target);
    match operator {
        Operator::Eq => ordering.is_eq(),
        Operator::Ne => ordering.is_ne(),
        Operator::Lt => ordering.is_lt(),
        Operator::Le => ordering.is_le(),
        Operator::Gt => ordering.is_gt(),
        Operator::Ge => ordering.is_ge(),
        _ => false,
    }
}

pub(super) fn passes(batch: &[TypedColumn], row: usize, branches: &[Vec<Condition>]) -> bool {
    branches.iter().any(|branch| {
        branch.iter().all(|condition| {
            let value = cell(&batch[condition.index], row);
            match &condition.test {
                Test::IsNull(negated) => matches!(value, Cell::Null) != *negated,
                Test::Like {
                    pattern,
                    case_insensitive,
                    negated,
                } => match &value {
                    Cell::Text(bytes) => {
                        like_matches(bytes, pattern, *case_insensitive) != *negated
                    }
                    _ => false,
                },
                Test::Compare(operator, target) => matches_comparison(&value, target, *operator),
                Test::In(targets, negated) => {
                    if matches!(value, Cell::Null) {
                        return false;
                    }
                    let present = targets
                        .iter()
                        .any(|target| matches_comparison(&value, target, Operator::Eq));
                    present != *negated
                }
                Test::NotBetween(lower, upper) => {
                    !matches!(value, Cell::Null)
                        && (!matches_comparison(&value, lower, Operator::Ge)
                            || !matches_comparison(&value, upper, Operator::Le))
                }
            }
        })
    })
}

pub(super) fn write_value<W: Write>(output: &mut W, value: &Cell) -> Result<(), ExecutionError> {
    match value {
        Cell::Numeric(value) => write!(output, "{value}")?,
        Cell::Null => output.write_all(b"\\N")?,
        Cell::Uuid(value) => output.write_all(value.as_bytes())?,
        Cell::Text(bytes) => {
            for &byte in bytes {
                match byte {
                    b'\\' => output.write_all(b"\\\\")?,
                    b'\t' => output.write_all(b"\\t")?,
                    b'\n' => output.write_all(b"\\n")?,
                    b'\r' => output.write_all(b"\\r")?,
                    0 => output.write_all(b"\\0")?,
                    0x01..=0x08 | 0x0b..=0x0c | 0x0e..=0x1f | 0x7f => {
                        write!(output, "\\x{byte:02X}")?;
                    }
                    other => output.write_all(&[other])?,
                }
            }
        }
    }
    Ok(())
}

fn write_row<W: Write>(output: &mut W, cells: &[Cell]) -> Result<(), ExecutionError> {
    for (index, cell) in cells.iter().enumerate() {
        if index != 0 {
            output.write_all(b"\t")?;
        }
        write_value(output, cell)?;
    }
    output.write_all(b"\n")?;
    Ok(())
}

struct Row {
    values: Vec<Cell>,
    sort_keys: Vec<Cell>,
}

// Keep the worst retained row at the root of the heap.
// Original arrival order breaks ties, matching stable ORDER BY.
struct TopRow {
    row: Row,
    ordinal: usize,
    descending_mask: u8,
    nulls_first_mask: u8,
    nulls_last_mask: u8,
}

fn compare_sort_keys(
    left: &[Cell],
    right: &[Cell],
    descending_mask: u8,
    nulls_first_mask: u8,
    nulls_last_mask: u8,
) -> Ordering {
    for (index, (a, b)) in left.iter().zip(right).enumerate() {
        let bit = 1u8 << index;
        let nulls_first = if nulls_first_mask & bit != 0 {
            Some(true)
        } else if nulls_last_mask & bit != 0 {
            Some(false)
        } else {
            None
        };
        let comparison = compare_order_cells(a, b, descending_mask & bit != 0, nulls_first);
        if comparison != Ordering::Equal {
            return comparison;
        }
    }
    Ordering::Equal
}

impl PartialEq for TopRow {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for TopRow {}

impl PartialOrd for TopRow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TopRow {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_sort_keys(
            &self.row.sort_keys,
            &other.row.sort_keys,
            self.descending_mask,
            self.nulls_first_mask,
            self.nulls_last_mask,
        )
        .then_with(|| self.ordinal.cmp(&other.ordinal))
    }
}

pub(super) fn execute_typed<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let projection = match &query.projection {
        Projection::All => metadata
            .columns
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>(),
        Projection::Columns(cols) => cols.clone(),
        Projection::Count => Vec::new(),
        Projection::CountColumn { column }
        | Projection::CountDistinct { column }
        | Projection::GroupedCount { column } => {
            vec![column.clone()]
        }
        _ => {
            return Err(ExecutionError::Invalid(
                "typed SQL currently supports SELECT columns and COUNT aggregates".into(),
            ));
        }
    };
    let mut names = projection.clone();
    let mut include = |name: &str| {
        if !names.iter().any(|value| value == name) {
            names.push(name.to_owned());
        }
    };
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        include(predicate.column());
    }
    let sort = if matches!(query.projection, Projection::GroupedCount { .. }) {
        Vec::new()
    } else {
        query
            .order_terms()
            .map(|order| query.order_source_column(&order.column).to_owned())
            .collect::<Vec<_>>()
    };
    for name in &sort {
        include(name);
    }
    if names.is_empty() {
        return Err(ExecutionError::Invalid(
            "typed scan requires a column".into(),
        ));
    }
    if query.distinct && sort.iter().any(|name| !projection.contains(name)) {
        return Err(ExecutionError::Invalid(
            "DISTINCT ORDER BY column must appear in SELECT".into(),
        ));
    }
    let indices: HashMap<String, usize> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let types: HashMap<String, String> = metadata
        .columns
        .iter()
        .map(|col| (col.name.clone(), col.data_type.clone()))
        .collect();
    let first = compile(&query.predicates, &indices, &types)?;
    let mut conditions = vec![first];
    for branch in &query.or_predicates {
        conditions.push(compile(branch, &indices, &types)?);
    }
    let projection_indices = projection
        .iter()
        .map(|name| indices[name])
        .collect::<Vec<_>>();
    let sort_indices = sort
        .iter()
        .map(|name| *indices.get(name).unwrap())
        .collect::<Vec<_>>();
    let mut scan = TypedQueryReader::open(part, &names)?;
    let header = match &query.projection {
        Projection::Count => "count".to_string(),
        Projection::CountColumn { column } => format!("count({column})"),
        Projection::CountDistinct { column } => format!("countDistinct({column})"),
        Projection::GroupedCount { column } => format!("{column}\tcount"),
        _ => projection.join("\t"),
    };
    super::write_header(query, output, &header)?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let is_count = matches!(query.projection, Projection::Count);
    let is_count_column = matches!(query.projection, Projection::CountColumn { .. });
    let is_count_distinct = matches!(query.projection, Projection::CountDistinct { .. });
    let is_grouped = matches!(query.projection, Projection::GroupedCount { .. });
    let mut count = 0u64;
    let mut distinct = HashSet::<Vec<Key>>::new();
    let mut group_indices = HashMap::<Key, usize>::new();
    let mut grouped_rows = Vec::<(Cell, u64)>::new();
    let mut ordered = Vec::<Row>::new();
    let mut top = BinaryHeap::<TopRow>::new();
    let mut ordinal = 0usize;
    let mut emitted = 0usize;
    let mut skipped = 0usize;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(usize::MAX);
    // ORDER BY supports at most eight keys, allowing a compact direction mask.
    let mut descending_mask = 0u8;
    let mut nulls_first_mask = 0u8;
    let mut nulls_last_mask = 0u8;
    for (index, order) in query.order_terms().enumerate() {
        let bit = 1u8 << index;
        if order.descending {
            descending_mask |= bit;
        }
        match order.nulls_first {
            Some(true) => nulls_first_mask |= bit,
            Some(false) => nulls_last_mask |= bit,
            None => {}
        }
    }
    let top_capacity = if sort_indices.is_empty() {
        None
    } else {
        query
            .limit
            .and_then(|count| count.checked_add(offset))
            .filter(|count| *count <= MAX_BUFFERED_ROWS)
    };
    while let Some(batch) = scan.next_batch()? {
        for index in 0..batch[0].len() {
            if !passes(&batch, index, &conditions) {
                continue;
            }
            if is_count || is_count_column {
                if is_count || !matches!(cell(&batch[projection_indices[0]], index), Cell::Null) {
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
                }
                continue;
            }
            if is_grouped {
                let value = cell(&batch[projection_indices[0]], index);
                match group_indices.entry(value.key()) {
                    Entry::Occupied(entry) => {
                        let count = &mut grouped_rows[*entry.get()].1;
                        *count = count.checked_add(1).ok_or_else(|| {
                            ExecutionError::Invalid("group count overflow".into())
                        })?;
                    }
                    Entry::Vacant(entry) => {
                        if grouped_rows.len() >= MAX_BUFFERED_ROWS {
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 100000 groups".into(),
                            ));
                        }
                        entry.insert(grouped_rows.len());
                        grouped_rows.push((value, 1));
                    }
                }
                continue;
            }
            let cells = projection_indices
                .iter()
                .map(|&idx| cell(&batch[idx], index))
                .collect::<Vec<_>>();
            if is_count_distinct && cells.iter().any(|value| matches!(value, Cell::Null)) {
                continue;
            }
            if is_count_distinct || query.distinct {
                let key = cells.iter().map(Cell::key).collect::<Vec<_>>();
                if distinct.len() >= MAX_BUFFERED_ROWS && !distinct.contains(&key) {
                    return Err(ExecutionError::Invalid("too many distinct rows".into()));
                }
                if !distinct.insert(key) {
                    continue;
                }
            }
            if is_count_distinct {
                continue;
            }
            if !sort_indices.is_empty() {
                let row = Row {
                    values: cells,
                    sort_keys: sort_indices
                        .iter()
                        .map(|&sort_index| cell(&batch[sort_index], index))
                        .collect(),
                };
                if let Some(capacity) = top_capacity {
                    let next = TopRow {
                        row,
                        ordinal,
                        descending_mask,
                        nulls_first_mask,
                        nulls_last_mask,
                    };
                    ordinal = ordinal
                        .checked_add(1)
                        .ok_or_else(|| ExecutionError::Invalid("row ordinal overflow".into()))?;
                    if top.len() < capacity {
                        top.push(next);
                    } else if top.peek().is_some_and(|worst| next < *worst) {
                        top.pop();
                        top.push(next);
                    }
                } else {
                    if ordered.len() >= MAX_BUFFERED_ROWS {
                        return Err(ExecutionError::Invalid(
                            "typed ORDER BY exceeds 100000 matching rows".into(),
                        ));
                    }
                    ordered.push(row);
                }
                continue;
            }
            if skipped < offset {
                skipped += 1;
                continue;
            }
            write_row(output, &cells)?;
            emitted += 1;
            if emitted == limit {
                return Ok(());
            }
        }
    }
    if is_grouped {
        if let Some(having) = &query.having {
            grouped_rows.retain(|(_, count)| {
                super::predicates::number_matches(
                    Number::Unsigned(u128::from(*count)),
                    Number::Unsigned(u128::from(having.value)),
                    having.operator,
                )
            });
        }
        if query.order_by.is_some() {
            grouped_rows.sort_by(|(left_key, left_count), (right_key, right_count)| {
                for order in query.order_terms() {
                    let comparison = if order.column == "count" {
                        let comparison = left_count.cmp(right_count);
                        if order.descending {
                            comparison.reverse()
                        } else {
                            comparison
                        }
                    } else {
                        compare_order_cells(
                            left_key,
                            right_key,
                            order.descending,
                            order.nulls_first,
                        )
                    };
                    if comparison != Ordering::Equal {
                        return comparison;
                    }
                }
                cmp(left_key, right_key)
            });
        }
        for (key, count) in grouped_rows.into_iter().skip(offset).take(limit) {
            write_value(output, &key)?;
            writeln!(output, "\t{count}")?;
        }
        return Ok(());
    }
    if is_count || is_count_column || is_count_distinct {
        if offset == 0 {
            let result = if is_count || is_count_column {
                count
            } else {
                distinct.len() as u64
            };
            writeln!(output, "{result}")?;
        }
        return Ok(());
    }
    if query.order_by.is_some() {
        if top_capacity.is_some() {
            for entry in top.into_sorted_vec().into_iter().skip(offset).take(limit) {
                write_row(output, &entry.row.values)?;
            }
        } else {
            ordered.sort_by(|left, right| {
                compare_sort_keys(
                    &left.sort_keys,
                    &right.sort_keys,
                    descending_mask,
                    nulls_first_mask,
                    nulls_last_mask,
                )
            });
            for row in ordered.into_iter().skip(offset).take(limit) {
                write_row(output, &row.values)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod like_tests {
    use super::{compile_like, like_matches};

    fn matches(text: &[u8], pattern: &str) -> bool {
        let compiled = compile_like(pattern).unwrap();
        like_matches(text, &compiled, false)
    }

    #[test]
    fn folds_ascii_case_but_keeps_escaping() {
        let pattern = compile_like("FOO\\_bAR%").unwrap();
        assert!(like_matches(b"foo_BarExtra", &pattern, true));
        assert!(!like_matches(b"foo_BarExtra", &pattern, false));
        assert!(like_matches(
            b"100%done",
            &compile_like("100\\%D%").unwrap(),
            true
        ));
        assert!(like_matches(
            b"AlphaBeta",
            &compile_like("a%%beTa").unwrap(),
            true
        ));
        assert!(!like_matches(b"foobarextra", &pattern, true));
    }

    #[test]
    fn supports_wildcards_and_backslash_escaping() {
        assert!(matches(b"api", "a_i"));
        assert!(matches(b"api", "%p%"));
        assert!(matches(b"", "%"));
        assert!(!matches(b"", "_"));
        assert!(matches("é".as_bytes(), "_"));
        assert!(matches("東京".as_bytes(), "__"));
        assert!(!matches("東京".as_bytes(), "_"));
        assert!(matches("aéz".as_bytes(), "a_z"));
        assert!(matches("é".as_bytes(), "%_"));
        assert!(matches(b"a_b", "a\\_b"));
        assert!(!matches(b"acb", "a\\_b"));
        assert!(matches(b"abc%d", "abc\\%d"));
        assert!(!matches(b"abcde", "abc\\%d"));
        assert!(matches(b"abcXYZend", "a%end"));
        assert!(matches(br"a\qb", r"a\qb"));
        assert!(!matches(b"aqb", r"a\qb"));
        assert!(!matches(b"abcXYZend", "a%ends"));
        assert!(compile_like("dangling\\").is_err());
    }
}
