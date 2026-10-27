//! Multiple grouped numeric aggregates over one typed scan.
use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError};
use crate::{AggregateFunction, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 100_000;

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    items: &[(AggregateFunction, String)],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut names = vec![column.to_owned()];
    for (_, value_column) in items {
        if !names.contains(value_column) {
            names.push(value_column.clone());
        }
        let selected = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == *value_column)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown aggregate column: {value_column}")))?;
        if matches!(typed::base_type(&selected.data_type), "String" | "UUID") {
            return Err(ExecutionError::Invalid("numeric aggregates require numeric columns".into()));
        }
    }
    for predicate in query.predicates.iter().chain(query.or_predicates.iter().flatten()) {
        if !names.contains(&predicate.column().to_owned()) {
            names.push(predicate.column().to_owned());
        }
    }
    let indices = names.iter().enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let types = metadata.columns.iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    let group_type = types.get(column).ok_or_else(|| {
        ExecutionError::Invalid(format!("unknown group column: {column}"))
    })?;
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    write!(output, "{column}")?;
    for (function, value_column) in items {
        write!(output, "\t{}({value_column})", function.name())?;
    }
    writeln!(output)?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let item_indices = items.iter().map(|(_, name)| indices[name]).collect::<Vec<_>>();
    let mut groups = Vec::<(Cell, Vec<AggregateState>)>::new();
    let mut by_key = HashMap::<typed::Key, usize>::new();
    let mut by_text = HashMap::<Vec<u8>, usize>::new();
    let mut reader = TypedQueryReader::open(part, &names)?;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let index = if let TypedColumn::Text(values) = &batch[indices[column]] {
                let key = values.get(row).ok_or_else(|| {
                    ExecutionError::Invalid("invalid String row index".into())
                })?;
                if let Some(&group) = by_text.get(key) {
                    group
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid("GROUP BY supports at most 100000 groups".into()));
                    }
                    let group = groups.len();
                    by_text.insert(key.to_vec(), group);
                    groups.push((Cell::Text(key.to_vec()), items.iter().map(|(function, _)| AggregateState::new(*function)).collect()));
                    group
                }
            } else {
                let key = typed::cell(&batch[indices[column]], row);
                match by_key.entry(key.key()) {
                    Entry::Occupied(entry) => *entry.get(),
                    Entry::Vacant(entry) => {
                        if groups.len() >= MAX_GROUPS {
                            return Err(ExecutionError::Invalid("GROUP BY supports at most 100000 groups".into()));
                        }
                        entry.insert(groups.len());
                        groups.push((key, items.iter().map(|(function, _)| AggregateState::new(*function)).collect()));
                        groups.len() - 1
                    }
                }
            };
            for (&column_index, state) in item_indices.iter().zip(groups[index].1.iter_mut()) {
                match typed::cell(&batch[column_index], row) {
                    Cell::Numeric(value) => state.update(value)?,
                    Cell::Null => {}
                    Cell::Text(_) | Cell::Uuid(_) => {
                        return Err(ExecutionError::Invalid("numeric aggregates require numeric columns".into()));
                    }
                }
            }
        }
    }
    if let Some(order) = &query.order_by {
        groups.sort_by(|(left, _), (right, _)| {
            typed::compare_order_cells(left, right, order.descending, order.nulls_first)
        });
    }
    for (key, states) in groups.into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        typed::write_typed_value(output, &key, group_type)?;
        for state in states {
            write!(output, "\t{}", state.render())?;
        }
        writeln!(output)?;
    }
    Ok(())
}
