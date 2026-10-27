use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError};
use crate::{Projection, Select};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, hash_map::Entry};
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

const MAX_GROUPS: usize = 100_000;

pub(super) fn execute_grouped_aggregate<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::GroupedAggregate {
        column,
        function,
        value_column,
    } = &query.projection
    else {
        unreachable!("grouped executor requires a grouped aggregate")
    };
    let metadata = inspect_query_source(part)?;
    let selected = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *value_column)
        .ok_or_else(|| {
            ExecutionError::Invalid(format!("unknown aggregate column: {value_column}"))
        })?;
    if matches!(typed::base_type(&selected.data_type), "String" | "UUID") {
        return Err(ExecutionError::Invalid(
            "numeric aggregates require a numeric column".into(),
        ));
    }

    let mut names = vec![column.clone()];
    if !names.contains(value_column) {
        names.push(value_column.clone());
    }
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
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    let group_index = indices[column];
    let value_index = indices[value_column];
    let mut reader = TypedQueryReader::open(part, &names)?;
    writeln!(output, "{column}\t{}({value_column})", function.name())?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let mut by_key = HashMap::<typed::Key, usize>::new();
    let mut groups = Vec::<(Cell, AggregateState)>::new();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let key = typed::cell(&batch[group_index], row);
            let value = typed::cell(&batch[value_index], row);
            let group = match by_key.entry(key.key()) {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(entry) => {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 100000 groups".into(),
                        ));
                    }
                    entry.insert(groups.len());
                    groups.push((key, AggregateState::new(*function)));
                    groups.len() - 1
                }
            };
            match value {
                Cell::Numeric(value) => groups[group].1.update(value)?,
                Cell::Null => {}
                Cell::Text(_) | Cell::Uuid(_) => {
                    return Err(ExecutionError::Invalid(
                        "numeric aggregates require a numeric column".into(),
                    ));
                }
            }
        }
    }
    if let Some(order) = &query.order_by {
        groups.sort_by(|(left, _), (right, _)| {
            let comparison = typed::cmp(left, right);
            if order.descending {
                comparison.reverse()
            } else {
                comparison
            }
        });
    }
    for (key, state) in groups
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        typed::write_value(output, &key)?;
        writeln!(output, "\t{}", state.render())?;
    }
    Ok(())
}
