mod arithmetic;

use self::arithmetic::*;

use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError, Number, NumberKey, number_at};
use crate::{AggregateItem, ScalarExpr, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

const MAX_DISTINCT_VALUES: usize = 100_000;

#[derive(Default)]
struct DistinctValues {
    other: HashSet<typed::Key>,
    text: HashSet<Vec<u8>>,
    uuids: HashSet<[u8; 16]>,
}

impl DistinctValues {
    fn len(&self) -> usize {
        self.other.len() + self.text.len() + self.uuids.len()
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

fn eval_number(
    expression: &BoundExpr,
    batch: &[TypedColumn],
    row: usize,
) -> Result<Option<Number>, ExecutionError> {
    match expression {
        BoundExpr::Column(index) => numeric_argument_at(batch, *index, row),
        BoundExpr::Integer(value) => Ok(Some(Number::Signed(i128::from(*value)))),
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
    let mut states = items
        .iter()
        .map(|item| match item.function {
            Some(function) => State::Numeric(AggregateState::new(function)),
            None if item.distinct => State::Distinct(DistinctValues::default()),
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
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
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
