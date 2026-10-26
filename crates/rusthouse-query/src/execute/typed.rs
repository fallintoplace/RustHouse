//! Bounded typed SELECT path for String, UUID and Nullable wide-part columns.
//! Keep the numeric-only executor unchanged for its optimized fast paths.
use super::{ExecutionError, Number, NumberKey, compare_numbers, number_at};
use crate::{Operator, Predicate, Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

const MAX_BUFFERED_ROWS: usize = 100_000;

#[derive(Clone)]
enum Cell {
    Numeric(Number),
    Text(Vec<u8>),
    Uuid(String),
    Null,
}

#[derive(PartialEq, Eq, Hash)]
enum Key {
    Numeric(NumberKey),
    Text(Vec<u8>),
    Uuid(String),
    Null,
}

impl Cell {
    fn key(&self) -> Key {
        match self {
            Self::Numeric(value) => Key::Numeric(NumberKey::from(*value)),
            Self::Text(bytes) => Key::Text(bytes.clone()),
            Self::Uuid(value) => Key::Uuid(value.clone()),
            Self::Null => Key::Null,
        }
    }
}

fn cell(column: &TypedColumn, row: usize) -> Cell {
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
        | Projection::CountDistinct { column }
        | Projection::GroupedCount { column } => names.push(column),
        _ => {}
    }
    for pred in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if matches!(pred, Predicate::IsNull { .. }) {
            return Ok(true);
        }
        names.push(pred.column());
    }
    if let Some(order) = &query.order_by {
        names.push(&order.column);
    }
    Ok(metadata
        .columns
        .iter()
        .any(|column| is_typed(&column.data_type) && names.contains(&column.name.as_str())))
}

fn base_type(data_type: &str) -> &str {
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

enum Test {
    Compare(Operator, Cell),
    In(Vec<Cell>, bool),
    NotBetween(Cell, Cell),
    IsNull(bool),
}

struct Condition {
    index: usize,
    test: Test,
}

fn compile(
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
                Predicate::In { values, .. } => Test::In(
                    values
                        .iter()
                        .map(|v| parse_literal(kind, v))
                        .collect::<Result<Vec<_>, _>>()?,
                    false,
                ),
                Predicate::NotIn { values, .. } => Test::In(
                    values
                        .iter()
                        .map(|v| parse_literal(kind, v))
                        .collect::<Result<Vec<_>, _>>()?,
                    true,
                ),
                Predicate::NotBetween { lower, upper, .. } => {
                    Test::NotBetween(parse_literal(kind, lower)?, parse_literal(kind, upper)?)
                }
                Predicate::IsNull { negated, .. } => Test::IsNull(*negated),
            };
            Ok(Condition { index, test })
        })
        .collect()
}

fn cmp(left: &Cell, right: &Cell) -> Ordering {
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

fn passes(batch: &[TypedColumn], row: usize, branches: &[Vec<Condition>]) -> bool {
    branches.iter().any(|branch| {
        branch.iter().all(|condition| {
            let value = cell(&batch[condition.index], row);
            match &condition.test {
                Test::IsNull(negated) => matches!(value, Cell::Null) != *negated,
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

fn write_value<W: Write>(output: &mut W, value: &Cell) -> Result<(), ExecutionError> {
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
    sort_key: Cell,
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
        Projection::CountDistinct { column } => vec![column.clone()],
        _ => {
            return Err(ExecutionError::Invalid(
                "typed SQL currently supports SELECT columns, COUNT(*) and COUNT(DISTINCT col)"
                    .into(),
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
    let sort = query.order_by.as_ref().map(|order| {
        if query.alias.as_deref() == Some(order.column.as_str()) && projection.len() == 1 {
            projection[0].clone()
        } else {
            order.column.clone()
        }
    });
    if let Some(sort) = &sort {
        include(sort);
    }
    if names.is_empty() {
        return Err(ExecutionError::Invalid(
            "typed scan requires a column".into(),
        ));
    }
    if query.distinct && sort.as_ref().is_some_and(|name| !projection.contains(name)) {
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
    let sort_index = sort.as_ref().map(|name| *indices.get(name).unwrap());
    let mut scan = TypedQueryReader::open(part, &names)?;
    let header = match &query.projection {
        Projection::Count => "count".to_string(),
        Projection::CountDistinct { column } => format!("countDistinct({column})"),
        _ => projection.join("\t"),
    };
    writeln!(output, "{}", query.alias.as_deref().unwrap_or(&header))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let is_count = matches!(query.projection, Projection::Count);
    let is_count_distinct = matches!(query.projection, Projection::CountDistinct { .. });
    let mut count = 0u64;
    let mut distinct = HashSet::<Vec<Key>>::new();
    let mut ordered = Vec::<Row>::new();
    let mut emitted = 0usize;
    let mut skipped = 0usize;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(usize::MAX);
    while let Some(batch) = scan.next_batch()? {
        for index in 0..batch[0].len() {
            if !passes(&batch, index, &conditions) {
                continue;
            }
            if is_count {
                count = count
                    .checked_add(1)
                    .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
                continue;
            }
            let cells = projection_indices
                .iter()
                .map(|&idx| cell(&batch[idx], index))
                .collect::<Vec<_>>();
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
            if let Some(sort_index) = sort_index {
                if ordered.len() >= MAX_BUFFERED_ROWS {
                    return Err(ExecutionError::Invalid(
                        "typed ORDER BY exceeds 100000 matching rows".into(),
                    ));
                }
                ordered.push(Row {
                    values: cells,
                    sort_key: cell(&batch[sort_index], index),
                });
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
    if is_count || is_count_distinct {
        if offset == 0 {
            let result = if is_count {
                count
            } else {
                distinct.len() as u64
            };
            writeln!(output, "{result}")?;
        }
        return Ok(());
    }
    if let Some(order) = &query.order_by {
        ordered.sort_by(|left, right| {
            let comparison = cmp(&left.sort_key, &right.sort_key);
            if order.descending {
                comparison.reverse()
            } else {
                comparison
            }
        });
        for row in ordered.into_iter().skip(offset).take(limit) {
            write_row(output, &row.values)?;
        }
    }
    Ok(())
}
