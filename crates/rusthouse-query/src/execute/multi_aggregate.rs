use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError, Number};
use crate::{AggregateItem, ScalarExpr, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

enum State {
    Count(u64),
    Distinct(HashSet<typed::Key>),
    Numeric(AggregateState),
}

fn checked_binary(left: Number, right: Number, subtract: bool) -> Result<Number, ExecutionError> {
    let op = |a: i128, b: i128| {
        if subtract {
            a.checked_sub(b)
        } else {
            a.checked_add(b)
        }
    };
    match (left, right) {
        (Number::Float(a), b) => {
            let b = match b {
                Number::Float(b) => b,
                Number::Signed(b) => b as f64,
                Number::Unsigned(b) => b as f64,
            };
            Ok(Number::Float(if subtract { a - b } else { a + b }))
        }
        (a, Number::Float(b)) => {
            let a = match a {
                Number::Float(a) => a,
                Number::Signed(a) => a as f64,
                Number::Unsigned(a) => a as f64,
            };
            Ok(Number::Float(if subtract { a - b } else { a + b }))
        }
        (Number::Unsigned(a), Number::Unsigned(b)) => {
            let value = if subtract {
                a.checked_sub(b)
            } else {
                a.checked_add(b)
            };
            value.map(Number::Unsigned).ok_or_else(|| {
                ExecutionError::Invalid("unsigned aggregate expression overflow".into())
            })
        }
        (a, b) => {
            let to_i128 = |number: Number| -> Result<i128, ExecutionError> {
                match number {
                    Number::Unsigned(number) => i128::try_from(number).map_err(|_| {
                        ExecutionError::Invalid("aggregate expression exceeds Int128".into())
                    }),
                    Number::Signed(number) => Ok(number),
                    Number::Float(_) => unreachable!(),
                }
            };
            op(to_i128(a)?, to_i128(b)?)
                .map(Number::Signed)
                .ok_or_else(|| ExecutionError::Invalid("aggregate expression overflow".into()))
        }
    }
}

fn checked_multiply(left: Number, right: Number) -> Result<Number, ExecutionError> {
    let as_float = |number: Number| match number {
        Number::Float(value) => value,
        Number::Signed(value) => value as f64,
        Number::Unsigned(value) => value as f64,
    };
    match (left, right) {
        (Number::Float(_), _) | (_, Number::Float(_)) => {
            Ok(Number::Float(as_float(left) * as_float(right)))
        }
        (Number::Unsigned(a), Number::Unsigned(b)) => {
            a.checked_mul(b).map(Number::Unsigned).ok_or_else(|| {
                ExecutionError::Invalid("unsigned aggregate expression overflow".into())
            })
        }
        (a, b) => {
            let signed = |number: Number| match number {
                Number::Signed(value) => Ok(value),
                Number::Unsigned(value) => i128::try_from(value).map_err(|_| {
                    ExecutionError::Invalid("aggregate expression exceeds Int128".into())
                }),
                Number::Float(_) => unreachable!(),
            };
            signed(a)?
                .checked_mul(signed(b)?)
                .map(Number::Signed)
                .ok_or_else(|| ExecutionError::Invalid("aggregate expression overflow".into()))
        }
    }
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
            indices.get(name).copied().ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown aggregate column: {name}"))
            })
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

fn eval_number(
    expression: &BoundExpr,
    batch: &[TypedColumn],
    row: usize,
) -> Result<Option<Number>, ExecutionError> {
    match expression {
        BoundExpr::Column(index) => match typed::cell(&batch[*index], row) {
            Cell::Numeric(value) => Ok(Some(value)),
            Cell::Null => Ok(None),
            _ => Err(ExecutionError::Invalid(
                "aggregate expression requires numeric columns".into(),
            )),
        },
        BoundExpr::Integer(value) => Ok(Some(Number::Signed(i128::from(*value)))),
        BoundExpr::AffineColumn {
            index,
            constant,
            subtract,
        } => match typed::cell(&batch[*index], row) {
            Cell::Numeric(value) => checked_binary(
                value,
                Number::Signed(i128::from(*constant)),
                *subtract,
            )
            .map(Some),
            Cell::Null => Ok(None),
            _ => Err(ExecutionError::Invalid(
                "aggregate expression requires numeric columns".into(),
            )),
        },
        BoundExpr::TextLength(index) => match typed::cell(&batch[*index], row) {
            Cell::Text(bytes) => Ok(Some(Number::Unsigned(bytes.len() as u128))),
            Cell::Null => Ok(None),
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
        let value = typed::cell(&batch[*index], row);
        return match value {
            Cell::Null => Ok(None),
            _ => Ok(Some(value.key())),
        };
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
        if !names.contains(&predicate.column().to_owned()) {
            names.push(predicate.column().to_owned());
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
            None if item.distinct => State::Distinct(HashSet::new()),
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
    let mut reader = TypedQueryReader::open(part, &names)?;
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
                        if let Some(key) = distinct_argument(expression, &batch, row)? {
                            if seen.len() >= 100_000 && !seen.contains(&key) {
                                return Err(ExecutionError::Invalid(
                                    "COUNT(DISTINCT) supports at most 100000 values".into(),
                                ));
                            }
                            seen.insert(key);
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
