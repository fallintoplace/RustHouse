//! Numeric aggregates over typed wide parts, including text filters and nullable values.
use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError};
use crate::{Projection, Select};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

pub(super) fn execute_typed_aggregate<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::Aggregate { function, column } = &query.projection else {
        unreachable!("typed aggregate requires an aggregate projection");
    };
    let metadata = inspect_query_source(part)?;
    let selected = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown aggregate column: {column}")))?;
    if matches!(typed::base_type(&selected.data_type), "String" | "UUID") {
        return Err(ExecutionError::Invalid(
            "numeric aggregates require a numeric column".into(),
        ));
    }

    let mut names = vec![column.clone()];
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !names.iter().any(|name| name == predicate.column()) {
            names.push(predicate.column().to_owned());
        }
    }
    let indices: HashMap<String, usize> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let types: HashMap<String, String> = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect();
    let first = typed::compile(&query.predicates, &indices, &types)?;
    let mut conditions = vec![first];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    let projected_index = indices[column];
    let mut reader = TypedQueryReader::open(part, &names)?;
    let header = format!("{}({column})", function.name());
    writeln!(output, "{}", query.alias.as_deref().unwrap_or(&header))?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let mut state = AggregateState::new(*function);
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            match typed::cell(&batch[projected_index], row) {
                Cell::Numeric(value) => state.update(value)?,
                Cell::Null => {}
                Cell::Text(_) | Cell::Uuid(_) => {
                    return Err(ExecutionError::Invalid(
                        "numeric aggregates require a numeric column".into(),
                    ));
                }
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{}", state.render())?;
    }
    Ok(())
}
