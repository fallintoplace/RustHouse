mod arithmetic;

use self::arithmetic::*;

use super::typed::{self, Cell};
use super::{AggregateState, BitmapDistinct32, ExecutionError, Number, NumberKey, number_at};
use crate::{AggregateItem, ScalarExpr, Select};
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

const MAX_DISTINCT_VALUES: usize = 100_000;

#[derive(Default)]
struct DistinctValues {
    other: HashSet<typed::Key>,
    text: HashSet<Vec<u8>>,
    uuids: HashSet<[u8; 16]>,
    // Multi-metric queries share the exact UInt32/Int32 bitmap fast path.
    bitmap32: Option<BitmapDistinct32>,
}

impl DistinctValues {
    fn len(&self) -> usize {
        self.other.len() + self.text.len() + self.uuids.len()
            + self.bitmap32.as_ref().map_or(0, |bitmap| bitmap.len() as usize)
    }

    fn insert_key(&mut self, key: typed::Key) -> Result<(), ExecutionError> {
        if self.len() >= MAX_DISTINCT_VALUES && !self.other.contains(&key) {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) supports at most 100000 values".into(),
            ));
        }
        self.other.insert(key);
        Ok(())
    }

    fn insert_text(&mut self, value: &[u8]) -> Result<(), ExecutionError> {
        // Avoid allocating for duplicate String keys across millions of rows.
        if self.text.contains(value) {
            return Ok(());
        }
        if self.len() >= MAX_DISTINCT_VALUES {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) supports at most 100000 values".into(),
            ));
        }
        self.text.insert(value.to_vec());
        Ok(())
    }

    fn insert_uuid(&mut self, value: [u8; 16]) -> Result<(), ExecutionError> {
        if self.len() >= MAX_DISTINCT_VALUES && !self.uuids.contains(&value) {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) supports at most 100000 values".into(),
            ));
        }
        self.uuids.insert(value);
        Ok(())
    }

    fn insert_column(&mut self, column: &TypedColumn, row: usize) -> Result<bool, ExecutionError> {
        if let Some(bitmap) = self.bitmap32.as_mut() {
            let value = match column {
                TypedColumn::Numeric(NumericColumn::UInt32(values)) => Some(values[row]),
                TypedColumn::Numeric(NumericColumn::Int32(values)) =>
                    Some(values[row] as u32),
                TypedColumn::NullableNumeric {
                    values: NumericColumn::UInt32(values), nulls,
                } => (nulls[row] == 0).then_some(values[row]),
                TypedColumn::NullableNumeric {
                    values: NumericColumn::Int32(values), nulls,
                } => (nulls[row] == 0).then_some(values[row] as u32),
                _ => return Err(ExecutionError::Invalid(
                    "32-bit distinct bitmap requires a 32-bit integer column".into(),
                )),
            };
            if let Some(key) = value {
                bitmap.insert(key)?;
            }
            return Ok(true);
        }
        match column {
            TypedColumn::Text(values) => {
                if let Some(value) = values.get(row) {
                    self.insert_text(value)?;
                }
            }
            TypedColumn::NullableText { values, nulls } => {
                if nulls[row] == 0 {
                    if let Some(value) = values.get(row) {
                        self.insert_text(value)?;
                    }
                }
            }
            TypedColumn::Uuid(values) => self.insert_uuid(*values[row].as_bytes())?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

enum State {
    Count(u64),
    Distinct(DistinctValues),
    Numeric(AggregateState),
}

// Resolve column names once per query rather than for every aggregate and row.
enum BoundExpr {
    Column(usize),
    TextLength(usize),
    Integer(i64),
    Float(f64),
    // Fast path for the affine expression family in ClickBench Q30.
    AffineColumn {
        index: usize,
        constant: i64,
        subtract: bool,
    },
    Add(Box<Self>, Box<Self>),
    Subtract(Box<Self>, Box<Self>),
    Multiply(Box<Self>, Box<Self>),
    Divide(Box<Self>, Box<Self>),
    Compare(Box<Self>, crate::Operator, Box<Self>),
}

impl BoundExpr {
    fn bind(
        expression: &ScalarExpr,
        indices: &HashMap<String, usize>,
    ) -> Result<Self, ExecutionError> {
        let index = |name: &str| {
            indices
                .get(name)
                .copied()
                .ok_or_else(|| ExecutionError::Invalid(format!("unknown aggregate column: {name}")))
        };
        Ok(match expression {
            ScalarExpr::Column(name) => Self::Column(index(name)?),
            ScalarExpr::TextLength(name) => Self::TextLength(index(name)?),
            ScalarExpr::Integer(number) => Self::Integer(*number),
            ScalarExpr::Decimal(number) => {
                let value = number.parse::<f64>().map_err(|_| {
                    ExecutionError::Invalid("invalid decimal expression literal".into())
                })?;
                if !value.is_finite() {
                    return Err(ExecutionError::Invalid("non-finite decimal literal".into()));
                }
                Self::Float(value)
            },
            ScalarExpr::Add(left, right) => {
                if let (ScalarExpr::Column(name), ScalarExpr::Integer(constant)) =
                    (left.as_ref(), right.as_ref())
                {
                    Self::AffineColumn {
                        index: index(name)?,
                        constant: *constant,
                        subtract: false,
                    }
                } else {
                    Self::Add(
                        Box::new(Self::bind(left, indices)?),
                        Box::new(Self::bind(right, indices)?),
                    )
                }
            }
            ScalarExpr::Subtract(left, right) => {
                if let (ScalarExpr::Column(name), ScalarExpr::Integer(constant)) =
                    (left.as_ref(), right.as_ref())
                {
                    Self::AffineColumn {
                        index: index(name)?,
                        constant: *constant,
                        subtract: true,
                    }
                } else {
                    Self::Subtract(
                        Box::new(Self::bind(left, indices)?),
                        Box::new(Self::bind(right, indices)?),
                    )
                }
            }
            ScalarExpr::Multiply(left, right) => Self::Multiply(
                Box::new(Self::bind(left, indices)?),
                Box::new(Self::bind(right, indices)?),
            ),
            ScalarExpr::Divide(left, right) => Self::Divide(
                Box::new(Self::bind(left, indices)?),
                Box::new(Self::bind(right, indices)?),
            ),
            ScalarExpr::Compare(left, operator, right) => Self::Compare(
                Box::new(Self::bind(left, indices)?),
                *operator,
                Box::new(Self::bind(right, indices)?),
            ),
        })
    }
}

fn numeric_argument_at(
    batch: &[TypedColumn],
    index: usize,
    row: usize,
) -> Result<Option<Number>, ExecutionError> {
    // Number extraction does not require allocating a String/UUID Cell.
    match &batch[index] {
        TypedColumn::Numeric(values) => Ok(Some(number_at(values, row))),
        TypedColumn::NullableNumeric { values, nulls } => {
            if nulls[row] != 0 {
                Ok(None)
            } else {
                Ok(Some(number_at(values, row)))
            }
        }
        _ => Err(ExecutionError::Invalid(
            "aggregate expression requires numeric columns".into(),
        )),
    }
}

/// Compare a UInt128 against an IEEE Float64 without rounding the integer
/// through Float64 first. Conversions below are exact for in-range whole
/// Float64 values; fractional portions are resolved after integer comparison.
fn compare_unsigned_float(integer: u128, float: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if float.is_nan() { return None; }
    if float < 0.0 { return Some(Ordering::Greater); }
    if float.is_infinite() || float >= 2.0_f64.powi(128) {
        return Some(Ordering::Less);
    }
    let whole = float.trunc() as u128;
    Some(integer.cmp(&whole).then_with(|| {
        if float.fract() > 0.0 { Ordering::Less } else { Ordering::Equal }
    }))
}

fn compare_signed_float(integer: i128, float: f64) -> Option<std::cmp::Ordering> {
    if integer >= 0 {
        compare_unsigned_float(integer as u128, float)
    } else if float >= 0.0 {
        Some(std::cmp::Ordering::Less)
    } else {
        compare_unsigned_float(integer.unsigned_abs(), -float)
            .map(std::cmp::Ordering::reverse)
    }
}

/// SQL expression comparisons must compare numeric values, not the
/// internal Number enum variant order used by mixed-type sort keys.
fn compare_expression_numbers(left: Number, right: Number) -> Option<std::cmp::Ordering> {
    match (left, right) {
        (Number::Unsigned(left), Number::Signed(right)) => Some(
            match u128::try_from(right) {
                Ok(right) => left.cmp(&right),
                Err(_) => std::cmp::Ordering::Greater,
            }
        ),
        (Number::Signed(left), Number::Unsigned(right)) => Some(
            match u128::try_from(left) {
                Ok(left) => left.cmp(&right),
                Err(_) => std::cmp::Ordering::Less,
            }
        ),
        (Number::Float(left), Number::Float(right)) => left.partial_cmp(&right),
        (Number::Unsigned(left), Number::Float(right)) => compare_unsigned_float(left, right),
        (Number::Signed(left), Number::Float(right)) => compare_signed_float(left, right),
        (Number::Float(left), Number::Unsigned(right)) => compare_unsigned_float(right, left)
            .map(std::cmp::Ordering::reverse),
        (Number::Float(left), Number::Signed(right)) => compare_signed_float(right, left)
            .map(std::cmp::Ordering::reverse),
        _ => Some(super::compare_numbers(left, right)),
    }
}

fn eval_number(
    expression: &BoundExpr,
    batch: &[TypedColumn],
    row: usize,
) -> Result<Option<Number>, ExecutionError> {
    match expression {
        BoundExpr::Column(index) => numeric_argument_at(batch, *index, row),
        BoundExpr::Compare(left, operator, right) => {
            let Some(left) = eval_number(left, batch, row)? else {return Ok(None)};
            let Some(right) = eval_number(right, batch, row)? else {return Ok(None)};
            let order = compare_expression_numbers(left, right);
            let matched = match operator {
                crate::Operator::Eq => order.is_some_and(|order| order.is_eq()),
                crate::Operator::Ne => order.is_none_or(|order| order.is_ne()),
                crate::Operator::Lt => order.is_some_and(|order| order.is_lt()),
                crate::Operator::Le => order.is_some_and(|order| order.is_le()),
                crate::Operator::Gt => order.is_some_and(|order| order.is_gt()),
                crate::Operator::Ge => order.is_some_and(|order| order.is_ge()),
                _ => return Err(ExecutionError::Invalid(
                    "unsupported numeric comparison operator".into())),
            };
            Ok(Some(Number::Unsigned(u128::from(matched))))
        },
        BoundExpr::Integer(value) => Ok(Some(Number::Signed(i128::from(*value)))),
        BoundExpr::Float(value) => Ok(Some(Number::Float(*value))),
        BoundExpr::AffineColumn {
            index,
            constant,
            subtract,
        } => match numeric_argument_at(batch, *index, row)? {
            Some(value) => checked_binary(
                value,
                Number::Signed(i128::from(*constant)),
                *subtract,
            ).map(Some),
            None => Ok(None),
        },
        BoundExpr::TextLength(index) => match &batch[*index] {
            // Length only needs the decoded byte span. Do not clone payloads.
            TypedColumn::Text(values) => Ok(values
                .get(row)
                .map(|bytes| Number::Unsigned(bytes.len() as u128))),
            TypedColumn::NullableText { values, nulls } => {
                if nulls[row] != 0 {
                    Ok(None)
                } else {
                    Ok(values
                        .get(row)
                        .map(|bytes| Number::Unsigned(bytes.len() as u128)))
                }
            }
            _ => Err(ExecutionError::Invalid(
                "length requires a String column".into(),
            )),
        },
        BoundExpr::Divide(left, right) => {
            let Some(left) = eval_number(left, batch, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_number(right, batch, row)? else {
                return Ok(None);
            };
            let to_float = |value: Number| match value {
                Number::Unsigned(number) => number as f64,
                Number::Signed(number) => number as f64,
                Number::Float(number) => number,
            };
            let divisor = to_float(right);
            if divisor == 0.0 {
                return Err(ExecutionError::Invalid(
                    "division by zero in aggregate expression".into(),
                ));
            }
            let quotient = to_float(left) / divisor;
            if !quotient.is_finite() {
                return Err(ExecutionError::Invalid(
                    "non-finite aggregate division".into(),
                ));
            }
            Ok(Some(Number::Float(quotient)))
        }
        BoundExpr::Multiply(left, right) => {
            let Some(left) = eval_number(left, batch, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_number(right, batch, row)? else {
                return Ok(None);
            };
            checked_multiply(left, right).map(Some)
        }
        BoundExpr::Add(left, right) | BoundExpr::Subtract(left, right) => {
            let Some(left) = eval_number(left, batch, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_number(right, batch, row)? else {
                return Ok(None);
            };
            checked_binary(left, right, matches!(expression, BoundExpr::Subtract(..))).map(Some)
        }
    }
}

fn distinct_argument(
    expression: &BoundExpr,
    batch: &[TypedColumn],
    row: usize,
) -> Result<Option<typed::Key>, ExecutionError> {
    if let BoundExpr::Column(index) = expression {
        let key = match &batch[*index] {
            TypedColumn::Numeric(values) => {
                Some(typed::Key::Numeric(NumberKey::from(number_at(values, row))))
            }
            TypedColumn::Text(values) => values
                .get(row)
                .map(|value| typed::Key::Text(value.to_vec())),
            TypedColumn::Uuid(values) => {
                Some(typed::Key::UuidBytes(*values[row].as_bytes()))
            },
            TypedColumn::NullableNumeric { values, nulls } => (nulls[row] == 0)
                .then(|| typed::Key::Numeric(NumberKey::from(number_at(values, row)))),
            TypedColumn::NullableText { values, nulls } => {
                if nulls[row] != 0 {
                    None
                } else {
                    values
                        .get(row)
                        .map(|value| typed::Key::Text(value.to_vec()))
                }
            }
        };
        return Ok(key);
    }
    Ok(eval_number(expression, batch, row)?.map(|value| Cell::Numeric(value).key()))
}

fn count_argument(
    argument: Option<&BoundExpr>,
    batch: &[TypedColumn],
    row: usize,
) -> Result<bool, ExecutionError> {
    match argument {
        None => Ok(true),
        Some(BoundExpr::Column(index)) => Ok(!typed::is_null_at(&batch[*index], row)),
        Some(expression) => Ok(eval_number(expression, batch, row)?.is_some()),
    }
}


/// Compute independent exact multi-aggregate partials over physical parts,
/// then merge the checked integer SUM and COUNT states. Other aggregate types
/// retain serial ordering to avoid changing floating-point semantics.
fn parallel_exact_multi_aggregates(
    part: &Path,
    names: &[String],
    items: &[AggregateItem],
    arguments: &[Option<BoundExpr>],
    conditions: &[Vec<typed::Condition>],
    no_filters: bool,
) -> Result<Option<Vec<State>>, ExecutionError> {
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    let chunk_size = parts.len().div_ceil(workers);
    let chunks = std::thread::scope(|scope| {
        let handles = parts.chunks(chunk_size).map(|paths| {
            scope.spawn(move || -> Result<Vec<State>, ExecutionError> {
                let mut states = items.iter().map(|item| match item.function {
                    Some(function) => State::Numeric(AggregateState::new(function)),
                    None => State::Count(0),
                }).collect::<Vec<_>>();
                for path in paths {
                    let mut reader = rusthouse_storage::TypedPartReader::open(path, names)?;
                    while let Some(batch) = reader.next_batch()? {
                        for row in 0..batch[0].len() {
                            if !no_filters && !typed::passes(&batch, row, conditions) {
                                continue;
                            }
                            for (state, argument) in states.iter_mut().zip(arguments) {
                                match state {
                                    State::Count(value) => {
                                        if count_argument(argument.as_ref(), &batch, row)? {
                                            *value = value.checked_add(1).ok_or_else(|| {
                                                ExecutionError::Invalid("COUNT overflow".into())
                                            })?;
                                        }
                                    }
                                    State::Numeric(value) => {
                                        let expr = argument.as_ref().ok_or_else(|| {
                                            ExecutionError::Invalid("aggregate argument missing".into())
                                        })?;
                                        if let Some(next) = eval_number(expr, &batch, row)? {
                                            value.update(next)?;
                                        }
                                    }
                                    State::Distinct(_) => unreachable!("only exact SUM and COUNT selected"),
                                }
                            }
                        }
                    }
                }
                Ok(states)
            })
        }).collect::<Vec<_>>();
        handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| {
            Err(ExecutionError::Invalid("parallel multi-aggregate worker panicked".into()))
        })).collect::<Vec<_>>()
    });
    let mut totals = items.iter().map(|item| match item.function {
        Some(function) => State::Numeric(AggregateState::new(function)),
        None => State::Count(0),
    }).collect::<Vec<_>>();
    for chunk in chunks {
        for (total, partial) in totals.iter_mut().zip(chunk?) {
            match (total, partial) {
                (State::Count(total), State::Count(partial)) => {
                    *total = total.checked_add(partial)
                        .ok_or_else(|| ExecutionError::Invalid("COUNT overflow".into()))?;
                }
                (State::Numeric(total), State::Numeric(partial)) => {
                    if let Some(value) = partial.value {
                        total.update(value)?;
                    }
                }
                _ => return Err(ExecutionError::Invalid(
                    "invalid parallel multi-aggregate state".into(),
                )),
            }
        }
    }
    Ok(Some(totals))
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    items: &[AggregateItem],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut names = Vec::new();
    for item in items {
        if let Some(expression) = &item.argument {
            expression.referenced_columns(&mut names);
        }
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let name = predicate.column();
        if !names.iter().any(|column| column == name) {
            names.push(name.to_owned());
        }
    }
    if names.is_empty() {
        names.push(
            metadata
                .columns
                .first()
                .ok_or_else(|| ExecutionError::Invalid("table has no columns".into()))?
                .name
                .clone(),
        );
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    for name in &names {
        if !types.contains_key(name) {
            return Err(ExecutionError::Invalid(format!("unknown column: {name}")));
        }
    }
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    writeln!(
        output,
        "{}",
        items
            .iter()
            .map(AggregateItem::header)
            .collect::<Vec<_>>()
            .join("\t")
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }
    // Non-nullable COUNT columns have exactly the snapshot's validated row
    // count. Answer any number of these metrics from metadata, without
    // opening or decompressing a single column stream.
    let metadata_counts = query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && items.iter().all(|item| {
            item.function.is_none()
                && !item.distinct
                && item.argument.as_ref().is_none_or(|expression| {
                    matches!(expression, ScalarExpr::Column(column)
                        if types.get(column).is_some_and(|kind|
                            !kind.starts_with("Nullable(")))
                })
        });
    if metadata_counts {
        if query.offset.unwrap_or(0) == 0 {
            for (index, _) in items.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\t")?;
                }
                write!(output, "{}", metadata.rows)?;
            }
            output.write_all(b"\n")?;
        }
        return Ok(());
    }
    // Mixed nullable COUNTs only need their compact null maps. A full
    // typed value scan decodes every numeric or String payload needlessly.
    // De-duplicate repeated nullable columns across up to 128 metrics.
    let nullmap_counts = query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && items.iter().all(|item| {
            item.function.is_none()
                && !item.distinct
                && item.argument.as_ref().is_none_or(|expression| {
                    matches!(expression, ScalarExpr::Column(column)
                        if types.contains_key(column))
                })
        });
    if nullmap_counts {
        let mut cached = HashMap::<String, u64>::new();
        for item in items {
            let Some(ScalarExpr::Column(column)) = &item.argument else {
                continue;
            };
            if !types[column].starts_with("Nullable(") || cached.contains_key(column) {
                continue;
            }
            let mut reader = TypedQueryReader::open_with_metadata(
                part, std::slice::from_ref(column), &metadata
            )?;
            let mut nonnull = 0u64;
            while let Some(nulls) = reader.next_null_batch()? {
                let count = nulls.iter().filter(|&&value| value == 0).count() as u64;
                nonnull = nonnull.checked_add(count).ok_or_else(|| {
                    ExecutionError::Invalid("COUNT overflow".into())
                })?;
            }
            cached.insert(column.clone(), nonnull);
        }
        if query.offset.unwrap_or(0) == 0 {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\t")?;
                }
                let count = match &item.argument {
                    Some(ScalarExpr::Column(column)) if types[column].starts_with("Nullable(") =>
                        cached[column],
                    _ => metadata.rows,
                };
                write!(output, "{count}")?;
            }
            output.write_all(b"\n")?;
        }
        return Ok(());
    }
    let mut states = items
        .iter()
        .map(|item| match item.function {
            Some(function) => State::Numeric(AggregateState::new(function)),
            None if item.distinct => {
                let bitmap32 = match item.argument.as_ref() {
                    Some(ScalarExpr::Column(column)) if types.get(column).is_some_and(|kind| {
                        matches!(typed::base_type(kind), "UInt32" | "Int32")
                    }) => Some(BitmapDistinct32::new()),
                    _ => None,
                };
                State::Distinct(DistinctValues {
                    bitmap32, ..DistinctValues::default()
                })
            },
            None => State::Count(0),
        })
        .collect::<Vec<_>>();
    let arguments = items
        .iter()
        .map(|item| {
            item.argument
                .as_ref()
                .map(|expression| BoundExpr::bind(expression, &indices))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let parallel_safe = items.iter().all(|item| match (&item.function, &item.argument, item.distinct) {
        (None, _, false) => true,
        (Some(crate::AggregateFunction::Sum), Some(crate::ScalarExpr::Column(column)), false) => {
            types.get(column).is_some_and(|kind| matches!(
                typed::base_type(kind),
                "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128"
                    | "Int8" | "Int16" | "Int32" | "Int64" | "Int128"
            ))
        }
        _ => false,
    });
    let parallel = if parallel_safe {
        parallel_exact_multi_aggregates(
            part, &names, items, &arguments, &conditions, no_filters,
        )?
    } else {
        None
    };
    let scan_serially = parallel.is_none();
    if let Some(partial) = parallel {
        states = partial;
    }
    let mut reader = if scan_serially {
        Some(TypedQueryReader::open_with_metadata(part, &names, &metadata)?)
    } else {
        None
    };
    while let Some(batch) = reader.as_mut().map_or(Ok(None), TypedQueryReader::next_batch)? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            for (state, argument) in states.iter_mut().zip(&arguments) {
                match state {
                    State::Count(value) => {
                        if count_argument(argument.as_ref(), &batch, row)? {
                            *value = value
                                .checked_add(1)
                                .ok_or_else(|| ExecutionError::Invalid("COUNT overflow".into()))?;
                        }
                    }
                    State::Distinct(seen) => {
                        let expression = argument.as_ref().ok_or_else(|| {
                            ExecutionError::Invalid("distinct argument missing".into())
                        })?;
                        if let BoundExpr::Column(index) = expression {
                            if seen.insert_column(&batch[*index], row)? {
                                continue;
                            }
                        }
                        if let Some(key) = distinct_argument(expression, &batch, row)? {
                            seen.insert_key(key)?;
                        }
                    }
                    State::Numeric(state) => {
                        let expression = argument.as_ref().ok_or_else(|| {
                            ExecutionError::Invalid("aggregate argument missing".into())
                        })?;
                        if let Some(value) = eval_number(expression, &batch, row)? {
                            state.update(value)?;
                        }
                    }
                }
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        for (index, state) in states.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\t")?;
            }
            match state {
                State::Count(count) => write!(output, "{count}")?,
                State::Distinct(seen) => write!(output, "{}", seen.len())?,
                State::Numeric(state) => write!(output, "{}", state.render())?,
            }
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

/// Compute exact arithmetic-series metrics in O(number of aggregates) instead
/// of iterating a billion generated rows. Predicated or DISTINCT inputs keep the
/// existing streaming path.
/// Only these generated metrics have a constant-time arithmetic result.
pub(super) fn supports_generated_series_metrics(items: &[AggregateItem]) -> bool {
    items.iter().all(|item| {
        !item.distinct
            && item.argument.as_ref().is_none_or(|expression| {
                matches!(expression, ScalarExpr::Column(name) if name == "number")
            })
            && matches!(item.function, None | Some(crate::AggregateFunction::Count
                | crate::AggregateFunction::Sum | crate::AggregateFunction::Min
                | crate::AggregateFunction::Max | crate::AggregateFunction::Avg))
    })
}

fn generated_series_metrics(
    start: u64,
    rows: u64,
    step: u64,
    items: &[AggregateItem],
) -> Option<Result<Vec<String>, ExecutionError>> {
    if !supports_generated_series_metrics(items) {
        return None;
    }
    let first = u128::from(start);
    let n = u128::from(rows);
    let last = if n == 0 {
        first
    } else {
        first + (n - 1) * u128::from(step)
    };
    let sum = if n & 1 == 0 {
        (n / 2) * (first + last)
    } else {
        n * ((first + last) / 2)
    };
    let results = items.iter().map(|item| {
        Ok(match item.function {
            None | Some(crate::AggregateFunction::Count) => rows.to_string(),
            Some(crate::AggregateFunction::Sum) if n > 0 => sum.to_string(),
            Some(crate::AggregateFunction::Min) if n > 0 => first.to_string(),
            Some(crate::AggregateFunction::Max) if n > 0 => last.to_string(),
            Some(crate::AggregateFunction::Avg) if n > 0 =>
                ((first as f64 + last as f64) / 2.0).to_string(),
            Some(crate::AggregateFunction::Sum | crate::AggregateFunction::Min
                | crate::AggregateFunction::Max | crate::AggregateFunction::Avg) => "NULL".into(),
            _ => unreachable!("fast path excludes other aggregate functions"),
        })
    }).collect::<Result<Vec<_>, ExecutionError>>();
    Some(results)
}

/// Evaluate bounded generated-number aggregate projections without a stored part.
pub(super) fn execute_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    items: &[AggregateItem],
    output: &mut W,
) -> Result<(), ExecutionError> {
    for item in items {
        if let Some(argument) = &item.argument {
            if !matches!(argument, ScalarExpr::Column(name) if name == "number") {
                return Err(ExecutionError::Invalid(
                    "generated aggregates support only the number column".into(),
                ));
            }
        } else if item.function.is_some() || item.distinct {
            return Err(ExecutionError::Invalid(
                "generated numeric aggregates require the number column".into(),
            ));
        }
    }
    let comparisons = super::compile_numbers_predicates(query)?;
    writeln!(
        output,
        "{}",
        items
            .iter()
            .map(AggregateItem::header)
            .collect::<Vec<_>>()
            .join("\t")
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }
    // Monotonic predicates select an exact contiguous arithmetic interval.
    // Reuse the formula path for the selected range instead of scanning every
    // generated row and updating each metric separately.
    let interval = if query.predicates.is_empty() && query.or_predicates.is_empty() {
        Some((0, rows))
    } else {
        super::monotonic_range_indices(&comparisons, start, rows, step)
    };
    if let Some((lower, upper)) = interval {
        let first = u128::from(start) + u128::from(lower) * u128::from(step);
        if let Ok(first) = u64::try_from(first) {
            if let Some(results) = generated_series_metrics(first, upper - lower, step, items) {
                if query.offset.unwrap_or(0) == 0 {
                    writeln!(output, "{}", results?.join("\t"))?;
                }
                return Ok(());
            }
        }
    }
    let mut states = items
        .iter()
        .map(|item| match item.function {
            Some(function) => State::Numeric(AggregateState::new(function)),
            None if item.distinct => State::Distinct(DistinctValues::default()),
            None => State::Count(0),
        })
        .collect::<Vec<_>>();
    for index in 0..rows {
        let value = Number::Unsigned(u128::from(start) + u128::from(index) * u128::from(step));
        if !super::number_matches_all(value, &comparisons) {
            continue;
        }
        for state in &mut states {
            match state {
                State::Count(count) => {
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| ExecutionError::Invalid("COUNT overflow".into()))?;
                }
                State::Distinct(seen) => {
                    seen.insert_key(typed::Key::Numeric(NumberKey::from(value)))?;
                }
                State::Numeric(state) => state.update(value)?,
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        for (index, state) in states.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\t")?;
            }
            match state {
                State::Count(count) => write!(output, "{count}")?,
                State::Distinct(seen) => write!(output, "{}", seen.len())?,
                State::Numeric(state) => write!(output, "{}", state.render())?,
            }
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod affine_tests {
    use super::*;

    #[test]
    fn binds_column_arithmetic_without_recursive_dispatch() {
        let indices = HashMap::from([("width".to_owned(), 0_usize)]);
        let column = || ScalarExpr::Column("width".into());
        let literal = || ScalarExpr::Integer(3);
        let add = ScalarExpr::Add(Box::new(column()), Box::new(literal()));
        let sub = ScalarExpr::Subtract(Box::new(column()), Box::new(literal()));

        assert!(matches!(
            BoundExpr::bind(&add, &indices).unwrap(),
            BoundExpr::AffineColumn {
                index: 0,
                constant: 3,
                subtract: false,
            }
        ));
        assert!(matches!(
            BoundExpr::bind(&sub, &indices).unwrap(),
            BoundExpr::AffineColumn {
                index: 0,
                constant: 3,
                subtract: true,
            }
        ));

        // Reversed addition must keep the generic path until it is proven safe.
        let reversed = ScalarExpr::Add(Box::new(literal()), Box::new(column()));
        assert!(matches!(
            BoundExpr::bind(&reversed, &indices).unwrap(),
            BoundExpr::Add(_, _)
        ));
    }
}

#[cfg(test)]
mod distinct_key_tests {
    use super::*;
    use rusthouse_storage::read_legacy_strings;

    #[test]
    fn distinct_state_uses_borrowed_text_and_fixed_width_uuid_keys() {
        let mut input = b"\x03abc\x03abc\x03xyz".as_slice();
        let values = read_legacy_strings(&mut input, 3).unwrap();
        let column = TypedColumn::Text(values);
        let mut distinct = DistinctValues::default();
        for row in 0..3 {
            assert!(distinct.insert_column(&column, row).unwrap());
        }
        assert_eq!(distinct.len(), 2);
        assert!(distinct.other.is_empty());
        assert_eq!(distinct.text.len(), 2);

        let nullable = TypedColumn::NullableNumeric {
            values: rusthouse_storage::NumericColumn::UInt64(vec![1]),
            nulls: vec![1],
        };
        assert!(!distinct.insert_column(&nullable, 0).unwrap());
    }

    #[test]
    fn distinct_argument_preserves_nullable_string_values() {
        let mut stream = b"\x03abc\x03abc".as_slice();
        let strings = read_legacy_strings(&mut stream, 2).unwrap();
        let batch = vec![TypedColumn::NullableText {
            values: strings,
            nulls: vec![0, 1],
        }];
        let expression = BoundExpr::Column(0);
        assert!(matches!(
            distinct_argument(&expression, &batch, 0).unwrap(),
            Some(typed::Key::Text(value)) if value == b"abc"
        ));
        assert!(distinct_argument(&expression, &batch, 1).unwrap().is_none());
    }
}

#[cfg(test)]
mod tpch_decimal_literal_tests {
    use super::*;

    #[test]
    fn binds_and_evaluates_decimal_constant_once() {
        let bound = BoundExpr::bind(
            &ScalarExpr::Decimal("0.05".to_owned()), &HashMap::new()
        ).unwrap();
        let value = eval_number(&bound, &[], 0).unwrap();
        assert!(matches!(value, Some(Number::Float(f)) if (f - 0.05).abs() < 1e-15));
    }
}

#[cfg(test)]
mod direct_numeric_argument_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn evaluates_numeric_columns_and_preserves_nulls() {
        let batch = vec![
            TypedColumn::Numeric(NumericColumn::UInt64(vec![100, 200])),
            TypedColumn::NullableNumeric {
                values: NumericColumn::Int64(vec![-5, 7]),
                nulls: vec![0, 1],
            },
        ];
        assert!(matches!(eval_number(&BoundExpr::Column(0), &batch, 1).unwrap(),
            Some(Number::Unsigned(200))));
        assert!(matches!(eval_number(&BoundExpr::Column(1), &batch, 0).unwrap(),
            Some(Number::Signed(-5))));
        assert!(eval_number(&BoundExpr::Column(1), &batch, 1).unwrap().is_none());
        let affine = BoundExpr::AffineColumn { index: 0, constant: 5, subtract: false };
        assert!(matches!(eval_number(&affine, &batch, 0).unwrap(), Some(Number::Unsigned(105)) | Some(Number::Signed(105))));
    }
}

#[cfg(test)]
mod mixed_integer_expression_comparison_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn unsigned_columns_compare_numerically_against_signed_literals() {
        let batch = [TypedColumn::Numeric(NumericColumn::UInt64(vec![0, 2, 3]))];
        let expression = BoundExpr::Compare(
            Box::new(BoundExpr::Column(0)),
            crate::Operator::Gt,
            Box::new(BoundExpr::Integer(2)),
        );
        for (row, expected) in [(0, 0u128), (1, 0), (2, 1)] {
            assert!(matches!(eval_number(&expression, &batch, row).unwrap(),
                Some(Number::Unsigned(value)) if value == expected));
        }
        assert_eq!(compare_expression_numbers(
            Number::Unsigned(u128::MAX), Number::Signed(-1)
        ), Some(std::cmp::Ordering::Greater));
        assert_eq!(compare_expression_numbers(
            Number::Signed(i128::MIN), Number::Unsigned(0)
        ), Some(std::cmp::Ordering::Less));
    }
}

#[cfg(test)]
mod ieee_comparison_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn compares_float_zero_normally_and_nan_as_unordered() {
        assert_eq!(compare_expression_numbers(
            Number::Float(-0.0), Number::Float(0.0)),
            Some(std::cmp::Ordering::Equal));
        assert_eq!(compare_expression_numbers(
            Number::Float(f64::NAN), Number::Float(f64::NAN)), None);
        let batch = [
            TypedColumn::Numeric(NumericColumn::Float64(vec![-0.0, f64::NAN])),
            TypedColumn::Numeric(NumericColumn::Float64(vec![0.0, f64::NAN])),
        ];
        for (operator, row, expected) in [
            (crate::Operator::Eq, 0, 1),
            (crate::Operator::Ne, 0, 0),
            (crate::Operator::Eq, 1, 0),
            (crate::Operator::Ne, 1, 1),
            (crate::Operator::Lt, 1, 0),
            (crate::Operator::Ge, 1, 0),
        ] {
            let expression = BoundExpr::Compare(
                Box::new(BoundExpr::Column(0)), operator,
                Box::new(BoundExpr::Column(1)),
            );
            assert!(matches!(eval_number(&expression, &batch, row).unwrap(),
                Some(Number::Unsigned(actual)) if actual == expected));
        }
    }
}

#[cfg(test)]
mod exact_mixed_float_integer_comparison_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;
    use std::cmp::Ordering;

    #[test]
    fn integer_float_comparisons_keep_u128_precision() {
        let border = 9_007_199_254_740_992u128;
        assert_eq!(compare_unsigned_float(border + 1, border as f64), Some(Ordering::Greater));
        assert_eq!(compare_unsigned_float(u128::MAX, 2.0_f64.powi(128)), Some(Ordering::Less));
        assert_eq!(compare_unsigned_float(u128::MAX, f64::INFINITY), Some(Ordering::Less));
        assert_eq!(compare_unsigned_float(0, -0.0), Some(Ordering::Equal));
        assert_eq!(compare_signed_float(-1, -1.5), Some(Ordering::Greater));
        assert_eq!(compare_signed_float(i128::MIN, -2.0_f64.powi(127)), Some(Ordering::Equal));
        assert_eq!(compare_signed_float(-1, f64::NEG_INFINITY), Some(Ordering::Greater));
        assert_eq!(compare_unsigned_float(1, f64::NAN), None);
        assert_eq!(compare_expression_numbers(
            Number::Float(-1.5), Number::Signed(-1)), Some(Ordering::Less));
    }

    #[test]
    fn float_columns_compare_against_signed_constant_literals() {
        let batch = [TypedColumn::Numeric(NumericColumn::Float64(
            vec![-1.0, 3.0, 4.5, f64::NAN]
        ))];
        let comparison = BoundExpr::Compare(
            Box::new(BoundExpr::Column(0)),
            crate::Operator::Gt,
            Box::new(BoundExpr::Integer(3)),
        );
        for (index, expected) in [0u128, 0, 1, 0].into_iter().enumerate() {
            assert!(matches!(eval_number(&comparison, &batch, index).unwrap(),
                Some(Number::Unsigned(actual)) if actual == expected));
        }
    }
}
