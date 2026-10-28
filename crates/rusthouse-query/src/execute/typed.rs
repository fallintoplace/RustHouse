//! Bounded typed SELECT path for String, UUID and Nullable wide-part columns.
//! Keep the numeric-only executor unchanged for its optimized fast paths.

mod like;

use self::like::*;

use super::{ExecutionError, Number, NumberKey, compare_numbers, number_at};
use crate::{Operator, Predicate, Projection, Select};
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_BUFFERED_ROWS: usize = 200_000;

fn remember_distinct_text(seen: &mut HashSet<Vec<u8>>, value: &[u8]) -> Result<(), ExecutionError> {
    if seen.contains(value) {
        return Ok(());
    }
    if seen.len() >= MAX_BUFFERED_ROWS {
        return Err(ExecutionError::Invalid("too many distinct rows".into()));
    }
    seen.insert(value.to_vec());
    Ok(())
}

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
    UuidBytes([u8; 16]),
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

/// Build a grouped hash key directly from a typed batch slot.
/// Avoid creating an owned Cell only to clone its String or UUID again.
pub(super) fn key_at(column: &TypedColumn, row: usize) -> Key {
    match column {
        TypedColumn::Numeric(values) => Key::Numeric(NumberKey::from(number_at(values, row))),
        TypedColumn::NullableNumeric { values, nulls } => {
            if nulls[row] != 0 {
                Key::Null
            } else {
                Key::Numeric(NumberKey::from(number_at(values, row)))
            }
        }
        TypedColumn::Text(values) => Key::Text(values.get(row).unwrap().to_vec()),
        TypedColumn::NullableText { values, nulls } => {
            if nulls[row] != 0 {
                Key::Null
            } else {
                Key::Text(values.get(row).unwrap().to_vec())
            }
        }
        TypedColumn::Uuid(values) => Key::Uuid(values[row].to_string()),
    }
}

/// Test the null bitmap without converting a value or allocating String/UUID cells.
pub(super) fn is_null_at(column: &TypedColumn, row: usize) -> bool {
    match column {
        TypedColumn::NullableNumeric { nulls, .. } | TypedColumn::NullableText { nulls, .. } => {
            nulls[row] != 0
        }
        _ => false,
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
    matches!(data_type, "String" | "UUID" | "Date" | "DateTime")
        || data_type.starts_with("Nullable(")
        || data_type.starts_with("Array(")
        || data_type.starts_with("Map(")
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
    // Most queries reference just a few columns. Keep the cheaper linear
    // probe for them, but use hashed membership for wide SELECT * plans.
    if names.len() < 16 {
        return Ok(metadata
            .columns
            .iter()
            .any(|column| is_typed(&column.data_type) && names.contains(&column.name.as_str())));
    }
    let referenced: HashSet<&str> = names.into_iter().collect();
    Ok(metadata
        .columns
        .iter()
        .any(|column| is_typed(&column.data_type) && referenced.contains(column.name.as_str())))
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
        "Date" => rusthouse_storage::parse_date(literal)
            .or_else(|| literal.parse::<u16>().ok())
            .map(|days| Cell::Numeric(Number::Unsigned(u128::from(days))))
            .ok_or_else(|| ExecutionError::Invalid("invalid Date literal".into())),
        "DateTime" => rusthouse_storage::parse_datetime(literal)
            .or_else(|| literal.parse::<u32>().ok())
            .map(|seconds| Cell::Numeric(Number::Unsigned(u128::from(seconds))))
            .ok_or_else(|| ExecutionError::Invalid("invalid DateTime literal".into())),
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

/// Decode a validated UUID literal once for binary comparisons during scans.
fn uuid_literal_bytes(text: &str) -> Result<[u8; 16], ExecutionError> {
    parse_literal("UUID", text)?;
    let mut output = [0u8; 16];
    let mut digits = text.bytes().filter(|byte| *byte != b'-');
    for byte in &mut output {
        let high = digits.next().expect("validated UUID").to_ascii_lowercase();
        let low = digits.next().expect("validated UUID").to_ascii_lowercase();
        let nibble = |value: u8| match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            _ => unreachable!("validated UUID"),
        };
        *byte = (nibble(high) << 4) | nibble(low);
    }
    Ok(output)
}

fn matches_uuid_compare(value: &[u8; 16], target: &[u8; 16], operator: Operator) -> bool {
    let ordering = value.cmp(target);
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

enum LikePattern {
    Exact(Vec<u8>),
    Prefix(Vec<u8>),
    Suffix(Vec<u8>),
    Contains(Vec<u8>),
    Wildcard(Vec<LikeToken>),
}

enum Test {
    Compare(Operator, Cell),
    UuidCompare(Operator, [u8; 16]),
    In(Vec<Cell>, bool),
    UuidInSet(HashSet<[u8; 16]>, bool),
    TextInSet(HashSet<Vec<u8>>, bool),
    NumericInSet(HashSet<NumberKey>, bool),
    NotBetween(Cell, Cell),
    UuidNotBetween([u8; 16], [u8; 16]),
    IsNull(bool),
    DistinctFrom(Cell, bool),
    Like {
        pattern: LikePattern,
        case_insensitive: bool,
        negated: bool,
    },
}

pub(super) struct Condition {
    index: usize,
    test: Test,
}

const HASHED_TYPED_IN_MIN_VALUES: usize = 16;

/// Convert already-validated canonical UUID literals into fixed-width hash keys.
fn uuid_key(value: &str) -> [u8; 16] {
    let mut digits = value.bytes().filter(|&byte| byte != b'-');
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => unreachable!("UUID literal validated during compilation"),
    };
    let mut bytes = [0u8; 16];
    for byte in &mut bytes {
        let high = digits.next().expect("validated UUID hex digits");
        let low = digits.next().expect("validated UUID hex digits");
        *byte = (nibble(high) << 4) | nibble(low);
    }
    bytes
}

fn compile_in(kind: &str, values: &[String], negated: bool) -> Result<Test, ExecutionError> {
    let targets = parse_literals(kind, values)?;
    if base_type(kind) == "UUID" {
        return Ok(Test::UuidInSet(
            targets
                .into_iter()
                .filter_map(|value| match value {
                    Cell::Uuid(text) => Some(uuid_key(&text)),
                    _ => None,
                })
                .collect(),
            negated,
        ));
    }
    if targets.len() < HASHED_TYPED_IN_MIN_VALUES {
        return Ok(Test::In(targets, negated));
    }
    if base_type(kind) == "String" {
        return Ok(Test::TextInSet(
            targets
                .into_iter()
                .filter_map(|cell| match cell {
                    Cell::Text(value) => Some(value),
                    _ => None,
                })
                .collect(),
            negated,
        ));
    }
    Ok(Test::NumericInSet(
        targets
            .into_iter()
            .filter_map(|cell| match cell {
                Cell::Numeric(value) => Some(NumberKey::from(value)),
                _ => None,
            })
            .collect(),
        negated,
    ))
}

fn order_compiled_conditions(conditions: &mut [Condition]) {
    // Reordering AND predicates is safe after compilation, and lets cheap
    // comparisons reject rows before expensive wildcard matching.
    conditions.sort_by_key(|condition| matches!(&condition.test, Test::Like { .. }));
}

pub(super) fn compile(
    predicates: &[Predicate],
    indices: &HashMap<String, usize>,
    types: &HashMap<String, String>,
) -> Result<Vec<Condition>, ExecutionError> {
    let mut conditions = predicates
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
                } if base_type(kind) == "UUID" => {
                    Test::UuidCompare(*operator, uuid_literal_bytes(value)?)
                }
                Predicate::Comparison {
                    operator, value, ..
                } => Test::Compare(*operator, parse_literal(kind, value)?),
                Predicate::In { values, .. } => compile_in(kind, values, false)?,
                Predicate::NotIn { values, .. } => compile_in(kind, values, true)?,
                Predicate::NotBetween { lower, upper, .. } if base_type(kind) == "UUID" => {
                    Test::UuidNotBetween(uuid_literal_bytes(lower)?, uuid_literal_bytes(upper)?)
                }
                Predicate::NotBetween { lower, upper, .. } => {
                    Test::NotBetween(parse_literal(kind, lower)?, parse_literal(kind, upper)?)
                }
                Predicate::IsNull { negated, .. } => Test::IsNull(*negated),
                Predicate::DistinctFrom { value, negated, .. } => {
                    Test::DistinctFrom(parse_literal(kind, value)?, *negated)
                }
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
        .collect::<Result<Vec<_>, _>>()?;
    order_compiled_conditions(&mut conditions);
    Ok(conditions)
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

pub(super) fn compare_order_cells(
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

// Avoid cloning String bytes for ordinary equality/range comparisons.
fn matches_comparison_at(
    column: &TypedColumn,
    row: usize,
    target: &Cell,
    operator: Operator,
) -> bool {
    let text = match column {
        TypedColumn::Text(values) => values.get(row),
        TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => values.get(row),
        TypedColumn::NullableText { .. } => return false,
        _ => return matches_comparison(&cell(column, row), target, operator),
    };
    let (Some(text), Cell::Text(target)) = (text, target) else {
        return false;
    };
    let order = text.cmp(target.as_slice());
    match operator {
        Operator::Eq => order.is_eq(),
        Operator::Ne => order.is_ne(),
        Operator::Lt => order.is_lt(),
        Operator::Le => order.is_le(),
        Operator::Gt => order.is_gt(),
        Operator::Ge => order.is_ge(),
        _ => false,
    }
}

// IN and NOT IN probe String bytes directly rather than cloning every row.
fn matches_in_at(column: &TypedColumn, row: usize, targets: &[Cell], negated: bool) -> bool {
    if matches!(
        column,
        TypedColumn::Text(_) | TypedColumn::NullableText { .. }
    ) {
        let text = match column {
            TypedColumn::Text(values) => values.get(row),
            TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => values.get(row),
            _ => None,
        };
        return text.is_some_and(|text| {
            targets
                .iter()
                .any(|target| matches!(target, Cell::Text(value) if text == value))
                != negated
        });
    }
    let value = cell(column, row);
    if matches!(value, Cell::Null) {
        return false;
    }
    targets
        .iter()
        .any(|target| matches_comparison(&value, target, Operator::Eq))
        != negated
}

pub(super) fn passes(batch: &[TypedColumn], row: usize, branches: &[Vec<Condition>]) -> bool {
    branches.iter().any(|branch| {
        branch.iter().all(|condition| {
            let column = &batch[condition.index];
            match &condition.test {
                // Filtering should not copy String payloads just to check their null map.
                Test::IsNull(negated) => {
                    is_null_at(column, row) != *negated
                }
                // LIKE examines bytes in place. Nulls never match, even for NOT LIKE.
                Test::Like {
                    pattern,
                    case_insensitive,
                    negated,
                } => {
                    let value = match column {
                        TypedColumn::Text(values) => values.get(row),
                        TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
                            values.get(row)
                        }
                        _ => None,
                    };
                    value.is_some_and(|bytes| {
                        like_matches(bytes, pattern, *case_insensitive) != *negated
                    })
                }
                Test::Compare(operator, target) => {
                    matches_comparison_at(column, row, target, *operator)
                }
                Test::UuidCompare(operator, target) => match column {
                    TypedColumn::Uuid(values) => {
                        matches_uuid_compare(values[row].as_bytes(), target, *operator)
                    }
                    _ => false,
                },
                Test::DistinctFrom(target, negated) => {
                    let equal = if let Cell::Numeric(super::Number::Float(right)) = target {
                        matches!(cell(column, row), Cell::Numeric(super::Number::Float(left)) if left.is_nan() && right.is_nan())
                            || matches_comparison_at(column, row, target, Operator::Eq)
                    } else {
                        matches_comparison_at(column, row, target, Operator::Eq)
                    };
                    equal == *negated
                }
                Test::In(targets, negated) => {
                    matches_in_at(column, row, targets, *negated)
                }
                Test::UuidInSet(targets, negated) => {
                    match column {
                        TypedColumn::Uuid(values) => {
                            targets.contains(values[row].as_bytes()) != *negated
                        }
                        _ => false,
                    }
                }
                Test::TextInSet(targets, negated) => {
                    let value = match column {
                        TypedColumn::Text(values) => values.get(row),
                        TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
                            values.get(row)
                        }
                        _ => None,
                    };
                    value.is_some_and(|bytes| targets.contains(bytes) != *negated)
                }
                Test::NumericInSet(targets, negated) => {
                    match cell(column, row) {
                        Cell::Numeric(value) => {
                            let found = !matches!(value, Number::Float(n) if n.is_nan())
                                && targets.contains(&NumberKey::from(value));
                            found != *negated
                        }
                        _ => false,
                    }
                }
                Test::UuidNotBetween(lower, upper) => match column {
                    TypedColumn::Uuid(values) => {
                        let value = values[row].as_bytes();
                        value < lower || value > upper
                    }
                    _ => false,
                },
                Test::NotBetween(lower, upper) => {
                    let is_null = match column {
                        TypedColumn::NullableNumeric { nulls, .. }
                        | TypedColumn::NullableText { nulls, .. } => nulls[row] != 0,
                        _ => false,
                    };
                    !is_null
                        && (!matches_comparison_at(column, row, lower, Operator::Ge)
                            || !matches_comparison_at(column, row, upper, Operator::Le))
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
            let mut start = 0;
            for (index, &byte) in bytes.iter().enumerate() {
                let escape: Option<&[u8]> = match byte {
                    b'\\' => Some(b"\\\\"),
                    b'\t' => Some(b"\\t"),
                    b'\n' => Some(b"\\n"),
                    b'\r' => Some(b"\\r"),
                    0 => Some(b"\\0"),
                    0x01..=0x08 | 0x0b..=0x0c | 0x0e..=0x1f | 0x7f => {
                        if start < index {
                            output.write_all(&bytes[start..index])?;
                        }
                        write!(output, "\\x{byte:02X}")?;
                        start = index + 1;
                        continue;
                    }
                    _ => None,
                };
                if let Some(escape) = escape {
                    if start < index {
                        output.write_all(&bytes[start..index])?;
                    }
                    output.write_all(escape)?;
                    start = index + 1;
                }
            }
            if start < bytes.len() {
                output.write_all(&bytes[start..])?;
            }
        }
    }
    Ok(())
}

pub(super) fn write_typed_value<W: Write>(
    output: &mut W,
    value: &Cell,
    data_type: &str,
) -> Result<(), ExecutionError> {
    match (base_type(data_type), value) {
        ("Date", Cell::Numeric(Number::Unsigned(days))) => {
            if let Ok(days) = u16::try_from(*days) {
                return write!(output, "{}", rusthouse_storage::format_date(days))
                    .map_err(ExecutionError::Io);
            }
        }
        ("DateTime", Cell::Numeric(Number::Unsigned(seconds))) => {
            if let Ok(seconds) = u32::try_from(*seconds) {
                return write!(output, "{}", rusthouse_storage::format_datetime(seconds))
                    .map_err(ExecutionError::Io);
            }
        }
        _ => {}
    }
    write_value(output, value)
}

fn write_row<W: Write>(
    output: &mut W,
    cells: &[Cell],
    data_types: &[&str],
) -> Result<(), ExecutionError> {
    for (index, cell) in cells.iter().enumerate() {
        if index != 0 {
            output.write_all(b"\t")?;
        }
        write_typed_value(output, cell, data_types[index])?;
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

/// Resolve IS NULL on non-nullable columns before opening typed streams.
/// A known-true disjunct dominates unknowns; false conjuncts are discarded.
fn static_nonnullable_null_filter(
    branches: &[Vec<Condition>],
    names: &[String],
    types: &HashMap<String, String>,
) -> Option<bool> {
    let mut unknown_branch = false;
    for branch in branches {
        let mut impossible = false;
        let mut unknown = false;
        for condition in branch {
            let kind = types.get(&names[condition.index])?;
            match &condition.test {
                Test::IsNull(negated) if !kind.starts_with("Nullable(") => {
                    if !*negated {
                        impossible = true;
                        break;
                    }
                }
                _ => unknown = true,
            }
        }
        if !impossible && !unknown {
            return Some(true);
        }
        if !impossible && unknown {
            unknown_branch = true;
        }
    }
    if unknown_branch { None } else { Some(false) }
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
    let projection_types = projection
        .iter()
        .map(|name| types.get(name).map(String::as_str).unwrap_or(""))
        .collect::<Vec<_>>();
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
    let header = match &query.projection {
        Projection::Count => "count".to_string(),
        Projection::CountColumn { column } => format!("count({column})"),
        Projection::CountDistinct { column } => format!("countDistinct({column})"),
        Projection::GroupedCount { column } => {
            format!("{column}\t{}", query.alias.as_deref().unwrap_or("count"))
        }
        _ => projection.join("\t"),
    };
    if matches!(query.projection, Projection::GroupedCount { .. }) {
        writeln!(output, "{header}")?;
    } else {
        super::write_header(query, output, &header)?;
    }
    if query.limit == Some(0) {
        return Ok(());
    }
    // A constant NULL test needs only the validated schema and row count.
    if matches!(query.projection, Projection::Count)
        && !query.distinct
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
        && query.having.is_none()
    {
        if let Some(all_rows) = static_nonnullable_null_filter(&conditions, &names, &types) {
            if query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{}", if all_rows { metadata.rows } else { 0 })?;
            }
            return Ok(());
        }
    }
    // COUNT(nullable_column) only needs the per-row NULL bit. Decode its
    // small null-map stream instead of materializing possibly huge payloads.
    if let Projection::CountColumn { column } = &query.projection {
        if query.predicates.is_empty()
            && query.or_predicates.is_empty()
            && !query.distinct
            && query.order_by.is_none()
            && query.order_by_extra.is_empty()
            && types
                .get(column)
                .is_some_and(|kind| kind.starts_with("Nullable("))
        {
            let mut reader = TypedQueryReader::open(part, std::slice::from_ref(column))?;
            let mut count = 0u64;
            while let Some(nulls) = reader.next_null_batch()? {
                count = count
                    .checked_add(nulls.iter().filter(|&&byte| byte == 0).count() as u64)
                    .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
            }
            if query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{count}")?;
            }
            return Ok(());
        }
    }
    // COUNT(non-nullable column) is identical to COUNT(*). Read the
    // snapshot row count rather than decoding String/UUID/numeric streams.
    if let Projection::CountColumn { column } = &query.projection {
        if query.predicates.is_empty()
            && query.or_predicates.is_empty()
            && !query.distinct
            && query.order_by.is_none()
            && query.order_by_extra.is_empty()
            && metadata.columns.iter().any(|candidate| {
                candidate.name == *column && !candidate.data_type.starts_with("Nullable(")
            })
        {
            if query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{}", metadata.rows)?;
            }
            return Ok(());
        }
    }
    let mut scan = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let is_count = matches!(query.projection, Projection::Count);
    let is_count_column = matches!(query.projection, Projection::CountColumn { .. });
    let is_count_distinct = matches!(query.projection, Projection::CountDistinct { .. });
    let is_grouped = matches!(query.projection, Projection::GroupedCount { .. });
    let mut count = 0u64;
    let mut distinct = HashSet::<Vec<Key>>::new();
    // A one-column DISTINCT key does not need a heap-allocated Vec<Key>.
    let mut single_distinct = HashSet::<Key>::new();
    let mut distinct_probe = Vec::<Key>::new();
    // COUNT(DISTINCT column) has one key; avoid a Vec allocation per row.
    let mut count_distinct_keys = HashSet::<Key>::new();
    // A nullable 16-bit domain fits into 8 KiB, independent of scan length.
    let mut count_distinct_sixteen = if is_count_distinct
        && matches!(
            projection_types.first().copied(),
            Some("UInt8" | "Int8" | "UInt16" | "Int16" | "Nullable(UInt8)" | "Nullable(Int8)" | "Nullable(UInt16)" | "Nullable(Int16)")
        )
    {
        Some(vec![0u64; 1024])
    } else {
        None
    };
    // Borrow decoded strings on duplicate probes. Own only new distinct values.
    let mut count_distinct_text = HashSet::<Vec<u8>>::new();
    // UUIDs have a fixed-size identity. Hash their bytes rather than allocating
    // a formatted String for each scanned row.
    let mut count_distinct_uuids = HashSet::<[u8; 16]>::new();
    let mut group_indices = HashMap::<Key, usize>::new();
    // String group probes can borrow the decoded input instead of cloning it.
    let mut text_group_indices = HashMap::<Vec<u8>, usize>::new();
    let mut uuid_group_indices = HashMap::<[u8; 16], usize>::new();
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
    // Allocate only the bounded set of winning rows. Avoid repeated heap
    // growth for typed ORDER BY while leaving unlimited scans lazy.
    if let Some(capacity) = top_capacity {
        top.reserve(capacity.min(1024));
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    if no_filters
        && !is_count
        && !is_count_column
        && !is_count_distinct
        && !is_grouped
        && !query.distinct
        && sort_indices.is_empty()
    {
        skipped = scan.skip_prefix_rows(offset)?;
    }
    while let Some(batch) = scan.next_batch()? {
        if no_filters && (is_count || is_count_column) {
            let matches = if is_count {
                batch[0].len()
            } else {
                match &batch[projection_indices[0]] {
                    TypedColumn::NullableNumeric { nulls, .. }
                    | TypedColumn::NullableText { nulls, .. } => {
                        nulls.iter().filter(|&&null| null == 0).count()
                    }
                    _ => batch[0].len(),
                }
            };
            count = count
                .checked_add(matches as u64)
                .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
            continue;
        }
        for index in 0..batch[0].len() {
            if !no_filters && !passes(&batch, index, &conditions) {
                continue;
            }
            if is_count || is_count_column {
                if is_count || !is_null_at(&batch[projection_indices[0]], index) {
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
                }
                continue;
            }
            if is_grouped {
                let text_key = match &batch[projection_indices[0]] {
                    TypedColumn::Text(strings) => strings.get(index),
                    TypedColumn::NullableText { values, nulls } if nulls[index] == 0 => {
                        values.get(index)
                    }
                    _ => None,
                };
                if let Some(key) = text_key {
                    if let Some(&group) = text_group_indices.get(key) {
                        let count = &mut grouped_rows[group].1;
                        *count = count.checked_add(1).ok_or_else(|| {
                            ExecutionError::Invalid("group count overflow".into())
                        })?;
                    } else {
                        if grouped_rows.len() >= MAX_BUFFERED_ROWS {
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        let group = grouped_rows.len();
                        text_group_indices.insert(key.to_vec(), group);
                        grouped_rows.push((Cell::Text(key.to_vec()), 1));
                    }
                    continue;
                }
                if let TypedColumn::Uuid(values) = &batch[projection_indices[0]] {
                    let key = *values[index].as_bytes();
                    if let Some(&group) = uuid_group_indices.get(&key) {
                        let count = &mut grouped_rows[group].1;
                        *count = count.checked_add(1).ok_or_else(|| {
                            ExecutionError::Invalid("group count overflow".into())
                        })?;
                    } else {
                        if grouped_rows.len() >= MAX_BUFFERED_ROWS {
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        let group = grouped_rows.len();
                        uuid_group_indices.insert(key, group);
                        grouped_rows.push((cell(&batch[projection_indices[0]], index), 1));
                    }
                    continue;
                }
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
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        entry.insert(grouped_rows.len());
                        grouped_rows.push((value, 1));
                    }
                }
                continue;
            }
            // In an ordinary unsorted scan, OFFSET can discard the row before
            // cloning String or UUID projection values.
            if !is_count_distinct && !query.distinct && sort_indices.is_empty() && skipped < offset
            {
                skipped += 1;
                continue;
            }
            // For non-DISTINCT top-k scans, sort keys determine whether a row
            // can enter the heap before we clone projected text or UUID values.
            let mut candidate_sort_keys = None;
            if !query.distinct
                && !is_count_distinct
                && !sort_indices.is_empty()
                && top_capacity.is_some_and(|capacity| capacity > 0 && top.len() >= capacity)
            {
                let keys = sort_indices
                    .iter()
                    .map(|&sort_index| cell(&batch[sort_index], index))
                    .collect::<Vec<_>>();
                if top.peek().is_some_and(|worst| {
                    compare_sort_keys(
                        &keys,
                        &worst.row.sort_keys,
                        descending_mask,
                        nulls_first_mask,
                        nulls_last_mask,
                    ) != Ordering::Less
                }) {
                    continue;
                }
                candidate_sort_keys = Some(keys);
            }
            if is_count_distinct {
                if let Some(bitmap) = count_distinct_sixteen.as_mut() {
                    let column = &batch[projection_indices[0]];
                    if is_null_at(column, index) {
                        continue;
                    }
                    let key = match column {
                        TypedColumn::Numeric(NumericColumn::UInt16(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::UInt16(values),
                            ..
                        } => values[index],
                        TypedColumn::Numeric(NumericColumn::Int16(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::Int16(values),
                            ..
                        } => values[index] as u16,
                        TypedColumn::Numeric(NumericColumn::UInt8(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::UInt8(values),
                            ..
                        } => u16::from(values[index]),
                        TypedColumn::Numeric(NumericColumn::Int8(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::Int8(values),
                            ..
                        } => values[index] as u16,
                        _ => {
                            return Err(ExecutionError::Invalid(
                                "unexpected 8-bit or 16-bit DISTINCT column type".into(),
                            ));
                        }
                    };
                    let index = usize::from(key);
                    bitmap[index / 64] |= 1u64 << (index % 64);
                    continue;
                }
                match &batch[projection_indices[0]] {
                    TypedColumn::Text(strings) => {
                        let value = strings.get(index).ok_or_else(|| {
                            ExecutionError::Invalid("invalid String row index".into())
                        })?;
                        remember_distinct_text(&mut count_distinct_text, value)?;
                    }
                    TypedColumn::NullableText { values, nulls } => {
                        if nulls[index] == 0 {
                            let value = values.get(index).ok_or_else(|| {
                                ExecutionError::Invalid("invalid String row index".into())
                            })?;
                            remember_distinct_text(&mut count_distinct_text, value)?;
                        }
                    }
                    TypedColumn::Uuid(values) => {
                        let key = *values[index].as_bytes();
                        if count_distinct_uuids.len() >= MAX_BUFFERED_ROWS {
                            if !count_distinct_uuids.contains(&key) {
                                return Err(ExecutionError::Invalid("too many distinct rows".into()));
                            }
                        } else {
                            count_distinct_uuids.insert(key);
                        }
                    }
                    column => {
                        let value = cell(column, index);
                        if !matches!(&value, Cell::Null) {
                            let key = value.key();
                            if count_distinct_keys.len() >= MAX_BUFFERED_ROWS {
                                if !count_distinct_keys.contains(&key) {
                                    return Err(ExecutionError::Invalid(
                                        "too many distinct rows".into(),
                                    ));
                                }
                            } else {
                                count_distinct_keys.insert(key);
                            }
                        }
                    }
                }
                continue;
            }
            if query.distinct {
                if projection_indices.len() == 1 {
                    // Avoid a second heap container around a single typed key.
                    if !single_distinct.insert(key_at(&batch[projection_indices[0]], index)) {
                        continue;
                    }
                    if single_distinct.len() > MAX_BUFFERED_ROWS {
                        return Err(ExecutionError::Invalid("too many distinct rows".into()));
                    }
                } else {
                    // Probe with just the hash key. Duplicate rows should never
                    // materialize a second set of projected String/UUID cells.
                    distinct_probe.clear();
                    for &idx in &projection_indices {
                        distinct_probe.push(key_at(&batch[idx], index));
                    }
                    if distinct.contains(distinct_probe.as_slice()) {
                        continue;
                    }
                    if distinct.len() >= MAX_BUFFERED_ROWS {
                        return Err(ExecutionError::Invalid("too many distinct rows".into()));
                    }
                    distinct.insert(std::mem::take(&mut distinct_probe));
                    distinct_probe = Vec::with_capacity(projection_indices.len());
                }
            }
            let cells = projection_indices
                .iter()
                .map(|&idx| cell(&batch[idx], index))
                .collect::<Vec<_>>();
            if !sort_indices.is_empty() {
                let row = Row {
                    values: cells,
                    sort_keys: candidate_sort_keys.unwrap_or_else(|| {
                        sort_indices
                            .iter()
                            .map(|&sort_index| cell(&batch[sort_index], index))
                            .collect()
                    }),
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
                        *top.peek_mut().unwrap() = next;
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
            write_row(output, &cells, &projection_types)?;
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
        let mut bounded_order = None;
        if query.order_by.is_some() {
            let compare = |(left_key, left_count): &(Cell, u64),
                           (right_key, right_count): &(Cell, u64)| {
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
            };
            let end = query.limit.and_then(|page| offset.checked_add(page));
            if let Some(end) = end.filter(|&end| end < grouped_rows.len()) {
                let mut indices = (0..grouped_rows.len()).collect::<Vec<_>>();
                let compare_index = |left: &usize, right: &usize| {
                    compare(&grouped_rows[*left], &grouped_rows[*right])
                        .then_with(|| left.cmp(right))
                };
                indices.select_nth_unstable_by(end, &compare_index);
                indices.truncate(end);
                indices.sort_unstable_by(compare_index);
                bounded_order = Some(indices);
            } else {
                grouped_rows.sort_by(compare);
            }
        }
        let write_group = |key: &Cell, count: u64, output: &mut W| -> Result<(), ExecutionError> {
            write_typed_value(output, key, projection_types[0])?;
            writeln!(output, "\t{count}")?;
            Ok(())
        };
        if let Some(indices) = bounded_order {
            for index in indices.into_iter().skip(offset) {
                write_group(&grouped_rows[index].0, grouped_rows[index].1, output)?;
            }
        } else {
            for (key, count) in grouped_rows.iter().skip(offset).take(limit) {
                write_group(key, *count, output)?;
            }
        }
        return Ok(());
    }
    if is_count || is_count_column || is_count_distinct {
        if offset == 0 {
            let result = if is_count || is_count_column {
                count
            } else {
                (count_distinct_keys.len()
                    + count_distinct_text.len()
                    + count_distinct_uuids.len()
                    + count_distinct_sixteen.as_ref().map_or(0, |bitmap| {
                        bitmap.iter().map(|bits| bits.count_ones() as usize).sum::<usize>()
                    })) as u64
            };
            writeln!(output, "{result}")?;
        }
        return Ok(());
    }
    if query.order_by.is_some() {
        if top_capacity.is_some() {
            for entry in top.into_sorted_vec().into_iter().skip(offset).take(limit) {
                write_row(output, &entry.row.values, &projection_types)?;
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
                write_row(output, &row.values, &projection_types)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod predicate_order_tests {
    use super::*;

    #[test]
    fn cheap_conditions_run_before_expensive_like_conditions() {
        let mut conditions = vec![
            Condition {
                index: 0,
                test: Test::Like {
                    pattern: LikePattern::Exact(b"match".to_vec()),
                    case_insensitive: false,
                    negated: false,
                },
            },
            Condition {
                index: 1,
                test: Test::IsNull(false),
            },
        ];
        order_compiled_conditions(&mut conditions);
        assert_eq!(conditions[0].index, 1);
        assert_eq!(conditions[1].index, 0);
    }
}

#[cfg(test)]
mod direct_group_key_tests {
    use super::{cell, key_at};
    use rusthouse_storage::{TypedColumn, read_legacy_strings};

    #[test]
    fn direct_group_keys_match_materialized_cells() {
        let mut input = b"\x03foo\x00\x03foo".as_slice();
        let values = read_legacy_strings(&mut input, 3).unwrap();
        let plain = TypedColumn::Text(values);
        for row in 0..3 {
            assert!(key_at(&plain, row) == cell(&plain, row).key());
        }

        let mut input = b"\x03foo\x00\x03bar".as_slice();
        let values = read_legacy_strings(&mut input, 3).unwrap();
        let nullable = TypedColumn::NullableText {
            values,
            nulls: vec![0, 1, 0],
        };
        for row in 0..3 {
            assert!(key_at(&nullable, row) == cell(&nullable, row).key());
        }
    }
}

#[cfg(test)]
mod like_tests {
    use super::{Condition, Test, compile_like, like_matches, passes};
    use rusthouse_storage::{NumericColumn, TypedColumn, read_legacy_strings};

    fn matches(text: &[u8], pattern: &str) -> bool {
        let compiled = compile_like(pattern).unwrap();
        like_matches(text, &compiled, false)
    }

    #[test]
    fn filters_borrowed_strings_and_nullable_rows() {
        let mut encoded: &[u8] = b"\x03foo\x03BAR\x03baz";
        let values = read_legacy_strings(&mut encoded, 3).unwrap();
        let batch = vec![
            TypedColumn::NullableText {
                values,
                nulls: vec![0, 1, 0],
            },
            TypedColumn::NullableNumeric {
                values: NumericColumn::UInt64(vec![2, 3, 4]),
                nulls: vec![0, 1, 0],
            },
        ];
        let like = vec![vec![Condition {
            index: 0,
            test: Test::Like {
                pattern: compile_like("F%").unwrap(),
                case_insensitive: true,
                negated: false,
            },
        }]];
        assert!(passes(&batch, 0, &like));
        assert!(!passes(&batch, 1, &like));
        assert!(!passes(&batch, 2, &like));

        let not_like = vec![vec![Condition {
            index: 0,
            test: Test::Like {
                pattern: compile_like("F%").unwrap(),
                case_insensitive: true,
                negated: true,
            },
        }]];
        assert!(!passes(&batch, 1, &not_like));
        assert!(passes(&batch, 2, &not_like));

        let null = vec![vec![Condition {
            index: 1,
            test: Test::IsNull(false),
        }]];
        assert!(!passes(&batch, 0, &null));
        assert!(passes(&batch, 1, &null));
        let not_null = vec![vec![Condition {
            index: 0,
            test: Test::IsNull(true),
        }]];
        assert!(passes(&batch, 0, &not_null));
        assert!(!passes(&batch, 1, &not_null));
    }

    #[test]
    fn compares_text_columns_without_copying_values() {
        use super::{Cell, matches_comparison_at};
        use crate::Operator;

        let mut encoded: &[u8] = b"\x03api\x02db";
        let values = read_legacy_strings(&mut encoded, 2).unwrap();
        let column = TypedColumn::NullableText {
            values,
            nulls: vec![0, 1],
        };
        let target = Cell::Text(b"api".to_vec());
        assert!(matches_comparison_at(&column, 0, &target, Operator::Eq));
        assert!(matches_comparison_at(
            &column,
            0,
            &Cell::Text(b"app".to_vec()),
            Operator::Lt
        ));
        assert!(!matches_comparison_at(&column, 0, &target, Operator::Ne));
        assert!(!matches_comparison_at(&column, 1, &target, Operator::Ne));
        assert!(!matches_comparison_at(
            &column,
            0,
            &Cell::Null,
            Operator::Eq
        ));
    }

    #[test]
    fn not_between_preserves_nullable_string_and_numeric_semantics() {
        use super::Cell;

        let mut encoded: &[u8] = b"\x03aaa\x03mmm\x03zzz";
        let values = read_legacy_strings(&mut encoded, 3).unwrap();
        let text = TypedColumn::NullableText {
            values,
            nulls: vec![0, 1, 0],
        };
        let number = TypedColumn::NullableNumeric {
            values: NumericColumn::UInt64(vec![2, 10, 99]),
            nulls: vec![0, 0, 1],
        };
        let rows = vec![text, number];
        let outside_text = vec![vec![Condition {
            index: 0,
            test: Test::NotBetween(Cell::Text(b"bbb".to_vec()), Cell::Text(b"yyy".to_vec())),
        }]];
        assert!(passes(&rows, 0, &outside_text));
        assert!(!passes(&rows, 1, &outside_text));
        assert!(passes(&rows, 2, &outside_text));
        let outside_numbers = vec![vec![Condition {
            index: 1,
            test: Test::NotBetween(
                Cell::Numeric(super::super::Number::Unsigned(2)),
                Cell::Numeric(super::super::Number::Unsigned(10)),
            ),
        }]];
        assert!(!passes(&rows, 0, &outside_numbers));
        assert!(!passes(&rows, 1, &outside_numbers));
        assert!(!passes(&rows, 2, &outside_numbers));
    }

    #[test]
    fn probes_string_in_lists_without_copying_and_preserves_null_semantics() {
        use super::{Cell, matches_in_at};

        let mut input: &[u8] = b"\x03api\x02db\x03web";
        let values = read_legacy_strings(&mut input, 3).unwrap();
        let column = TypedColumn::NullableText {
            values,
            nulls: vec![0, 1, 0],
        };
        let targets = vec![
            Cell::Text(b"api".to_vec()),
            Cell::Text(b"worker".to_vec()),
            Cell::Null,
        ];
        assert!(matches_in_at(&column, 0, &targets, false));
        assert!(!matches_in_at(&column, 0, &targets, true));
        assert!(!matches_in_at(&column, 1, &targets, false));
        assert!(!matches_in_at(&column, 1, &targets, true));
        assert!(!matches_in_at(&column, 2, &targets, false));
        assert!(matches_in_at(&column, 2, &targets, true));
    }

    #[test]
    fn hashes_long_in_lists_without_changing_nullable_semantics() {
        use super::{Test, compile_in};
        let words = (0..32).map(|i| format!("word{i}")).collect::<Vec<_>>();
        let text_test = compile_in("String", &words, false).unwrap();
        assert!(matches!(text_test, Test::TextInSet(_, false)));
        let not_text_test = compile_in("String", &words, true).unwrap();
        let mut input: &[u8] = b"\x05word7\x05other\x05word0";
        let strings = read_legacy_strings(&mut input, 3).unwrap();
        let col = TypedColumn::NullableText {
            values: strings,
            nulls: vec![0, 0, 1],
        };
        let batch = vec![col];
        let positive = vec![vec![Condition {
            index: 0,
            test: text_test,
        }]];
        let negative = vec![vec![Condition {
            index: 0,
            test: not_text_test,
        }]];
        assert!(passes(&batch, 0, &positive));
        assert!(!passes(&batch, 1, &positive));
        assert!(!passes(&batch, 2, &positive));
        assert!(!passes(&batch, 0, &negative));
        assert!(passes(&batch, 1, &negative));
        assert!(!passes(&batch, 2, &negative));

        let numbers = (0..32).map(|i| i.to_string()).collect::<Vec<_>>();
        let in_test = compile_in("UInt64", &numbers, false).unwrap();
        assert!(matches!(in_test, Test::NumericInSet(_, false)));
        let numeric = vec![TypedColumn::NullableNumeric {
            values: NumericColumn::UInt64(vec![7, 33, 0]),
            nulls: vec![0, 0, 1],
        }];
        let branch = vec![vec![Condition {
            index: 0,
            test: in_test,
        }]];
        assert!(passes(&numeric, 0, &branch));
        assert!(!passes(&numeric, 1, &branch));
        assert!(!passes(&numeric, 2, &branch));
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
    fn matches_simple_patterns_and_escaped_wildcards() {
        for (value, pattern, expected) in [
            (b"abc".as_slice(), "abc", true),
            (b"abcd".as_slice(), "abc", false),
            (b"abcd".as_slice(), "ab%", true),
            (b"abcd".as_slice(), "%cd", true),
            (b"abcd".as_slice(), "%bc%", true),
            (b"abcd".as_slice(), "%zz%", false),
            (b"abc".as_slice(), "%", true),
            (b"abc".as_slice(), "ab\\%c", false),
            (b"ab%c".as_slice(), "ab\\%c", true),
        ] {
            assert_eq!(matches(value, pattern), expected, "{pattern}");
        }
        let pattern = compile_like("%ABC%").unwrap();
        assert!(like_matches(b"xxabcyy", &pattern, true));
        assert!(!like_matches(b"xxabcyy", &pattern, false));
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

#[cfg(test)]
mod typed_output_perf_tests {
    use super::{Cell, write_value};
    use std::io::{self, Write};

    #[derive(Default)]
    struct CountingWriter {
        writes: usize,
        bytes: Vec<u8>,
    }

    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn preserves_tsv_escaping_across_plain_spans() {
        let mut output = Vec::new();
        write_value(
            &mut output,
            &Cell::Text(b"one\ttwo\\three\n\0\x01".to_vec()),
        )
        .unwrap();
        assert_eq!(output, br"one\ttwo\\three\n\0\x01");
    }

    #[test]
    fn emits_long_unescaped_strings_with_one_write() {
        let mut output = CountingWriter::default();
        write_value(&mut output, &Cell::Text(vec![b'a'; 16 * 1024])).unwrap();
        assert_eq!(output.bytes, vec![b'a'; 16 * 1024]);
        assert_eq!(output.writes, 1);
    }
}

#[cfg(test)]
mod unicode_ilike_regressions {
    use super::*;

    #[test]
    fn matches_non_ascii_letters_in_literal_and_simple_wildcard_patterns() {
        for (pattern, text) in [
            ("CAFÉ", "café"),
            ("CAFÉ%", "café au lait"),
            ("%CAFÉ", "un café"),
            ("%CAFÉ%", "un café noir"),
            ("İ", "i\u{307}"),
        ] {
            let compiled = compile_like(pattern).unwrap();
            assert!(like_matches(text.as_bytes(), &compiled, true), "{pattern}");
            assert!(
                !like_matches(text.as_bytes(), &compiled, false),
                "{pattern}"
            );
        }
    }

    #[test]
    fn keeps_ascii_behavior_and_does_not_match_invalid_utf8() {
        let pattern = compile_like("ABC%").unwrap();
        assert!(like_matches(b"abcdef", &pattern, true));
        assert!(!like_matches(&[0xff], &compile_like("É").unwrap(), true));
    }
}

#[cfg(test)]
mod topk_tests {
    use crate::{execute, parse_select};
    use rusthouse_storage::{Column, write_wide_part};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn bounded_sort_keeps_earlier_ties_and_skips_losing_rows() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-typed-topk-prune-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let columns = [
            Column {
                name: "note".into(),
                data_type: "String".into(),
            },
            Column {
                name: "rank".into(),
                data_type: "UInt64".into(),
            },
        ];
        let mut rows = vec![
            vec![Some("first".into()), Some("1".into())],
            vec![Some("second".into()), Some("1".into())],
        ];
        rows.extend((0..300).map(|index| vec![Some(format!("loser-{index}")), Some("99".into())]));
        write_wide_part(&path, &columns, &rows).unwrap();
        let query = parse_select("SELECT note FROM part ORDER BY rank LIMIT 2").unwrap();
        let mut output = Vec::new();
        execute(&query, Some(&path), &mut output).unwrap();
        assert_eq!(output, b"note\nfirst\nsecond\n");
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod nullable_string_group_tests {
    use crate::{execute, parse_select};
    use rusthouse_storage::{Column, write_wide_part};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn groups_nullable_text_with_borrowed_non_null_keys() {
        let part = std::env::temp_dir().join(format!(
            "rusthouse-null-text-group-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let columns = [Column {
            name: "category".into(),
            data_type: "Nullable(String)".into(),
        }];
        let rows = [
            vec![Some("x".into())],
            vec![None],
            vec![Some("x".into())],
            vec![Some("y".into())],
            vec![None],
        ];
        write_wide_part(&part, &columns, &rows).unwrap();
        let query = parse_select("SELECT category, count(*) FROM part GROUP BY category").unwrap();
        let mut output = Vec::new();
        execute(&query, Some(&part), &mut output).unwrap();
        assert_eq!(output, b"category\tcount\nx\t2\n\\N\t2\ny\t1\n");
        std::fs::remove_dir_all(part).unwrap();
    }
}

#[cfg(test)]
mod single_column_distinct_perf_tests {
    use crate::{execute, parse_select};
    use rusthouse_storage::{Column, write_wide_part};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn single_string_distinct_preserves_first_seen_order() {
        let part = std::env::temp_dir().join(format!(
            "rusthouse-single-distinct-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        let columns = [Column {
            name: "word".into(),
            data_type: "String".into(),
        }];
        let rows = [
            vec![Some("alpha".into())],
            vec![Some("beta".into())],
            vec![Some("alpha".into())],
            vec![Some("gamma".into())],
            vec![Some("beta".into())],
        ];
        write_wide_part(&part, &columns, &rows).unwrap();
        let query = parse_select("SELECT DISTINCT word FROM part").unwrap();
        let mut output = Vec::new();
        execute(&query, Some(&part), &mut output).unwrap();
        assert_eq!(output, b"word\nalpha\nbeta\ngamma\n");
        std::fs::remove_dir_all(part).unwrap();
    }
}
