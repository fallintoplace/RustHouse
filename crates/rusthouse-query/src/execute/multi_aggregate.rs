use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError, Number};
use crate::{AggregateItem, ScalarExpr, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

enum State {
    Count(u64),
    Numeric(AggregateState),
}

fn checked_binary(
    left: Number,
    right: Number,
    subtract: bool,
) -> Result<Number, ExecutionError> {
    let op = |a: i128, b: i128| {
        if subtract { a.checked_sub(b) } else { a.checked_add(b) }
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
            let value = if subtract { a.checked_sub(b) } else { a.checked_add(b) };
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

fn eval_number(
    expression: &ScalarExpr,
    batch: &[TypedColumn],
    row: usize,
    indices: &HashMap<String, usize>,
) -> Result<Option<Number>, ExecutionError> {
    match expression {
        ScalarExpr::Column(name) => {
            let index = indices.get(name).ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown aggregate column: {name}"))
            })?;
            match typed::cell(&batch[*index], row) {
                Cell::Numeric(value) => Ok(Some(value)),
                Cell::Null => Ok(None),
                _ => Err(ExecutionError::Invalid(
                    "aggregate expression requires numeric columns".into(),
                )),
            }
        }
        ScalarExpr::Integer(value) => Ok(Some(Number::Signed(i128::from(*value)))),
        ScalarExpr::Add(left, right) | ScalarExpr::Subtract(left, right) => {
            let Some(left) = eval_number(left, batch, row, indices)? else {
                return Ok(None);
            };
            let Some(right) = eval_number(right, batch, row, indices)? else {
                return Ok(None);
            };
            checked_binary(left, right, matches!(expression, ScalarExpr::Subtract(..))).map(Some)
        }
    }
}

fn count_argument(
    argument: Option<&ScalarExpr>,
    batch: &[TypedColumn],
    row: usize,
    indices: &HashMap<String, usize>,
) -> Result<bool, ExecutionError> {
    match argument {
        None => Ok(true),
        Some(ScalarExpr::Column(name)) => {
            let index = indices.get(name).ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown aggregate column: {name}"))
            })?;
            Ok(!matches!(typed::cell(&batch[*index], row), Cell::Null))
        }
        Some(expression) => Ok(eval_number(expression, batch, row, indices)?.is_some()),
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
            None => State::Count(0),
        })
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(part, &names)?;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            for (item, state) in items.iter().zip(&mut states) {
                match state {
                    State::Count(value) => {
                        if count_argument(item.argument.as_ref(), &batch, row, &indices)? {
                            *value = value.checked_add(1).ok_or_else(|| {
                                ExecutionError::Invalid("COUNT overflow".into())
                            })?;
                        }
                    }
                    State::Numeric(state) => {
                        let expression = item.argument.as_ref().ok_or_else(|| {
                            ExecutionError::Invalid("aggregate argument missing".into())
                        })?;
                        if let Some(value) = eval_number(expression, &batch, row, &indices)? {
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
                State::Numeric(state) => write!(output, "{}", state.render())?,
            }
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}
