//! Numeric aggregates over typed wide parts, including text filters and nullable values.
use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError};
use crate::{AggregateFunction, Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
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
    let ordered_text = matches!(typed::base_type(&selected.data_type), "String" | "UUID");
    if ordered_text && !matches!(function, AggregateFunction::Min | AggregateFunction::Max) {
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
    let header = format!("{}({column})", function.name());
    writeln!(output, "{}", query.alias.as_deref().unwrap_or(&header))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let mut reader = TypedQueryReader::open(part, &names)?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();

    let mut state = AggregateState::new(*function);
    let mut ordered_value: Option<Cell> = None;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            match typed::cell(&batch[projected_index], row) {
                Cell::Numeric(value) => state.update(value)?,
                Cell::Null => {}
                value @ (Cell::Text(_) | Cell::Uuid(_)) if ordered_text => {
                    let replace = ordered_value.as_ref().is_none_or(|current| {
                        let ordering = typed::cmp(&value, current);
                        match function {
                            AggregateFunction::Min => ordering.is_lt(),
                            AggregateFunction::Max => ordering.is_gt(),
                            _ => false,
                        }
                    });
                    if replace {
                        ordered_value = Some(value);
                    }
                }
                Cell::Text(_) | Cell::Uuid(_) => {
                    return Err(ExecutionError::Invalid(
                        "numeric aggregates require a numeric column".into(),
                    ));
                }
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        if ordered_text {
            if let Some(value) = ordered_value {
                typed::write_typed_value(output, &value, &selected.data_type)?;
                writeln!(output)?;
            } else {
                writeln!(output, "NULL")?;
            }
            return Ok(());
        }
        let rendered = state.render();
        let rendered = match (function, selected.data_type.as_str()) {
            (AggregateFunction::Min | AggregateFunction::Max, "Date") => rendered
                .parse::<u16>()
                .map(rusthouse_storage::format_date)
                .unwrap_or(rendered),
            (AggregateFunction::Min | AggregateFunction::Max, "DateTime") => rendered
                .parse::<u32>()
                .map(rusthouse_storage::format_datetime)
                .unwrap_or(rendered),
            _ => rendered,
        };
        writeln!(output, "{rendered}")?;
    }
    Ok(())
}

const MAX_GROUPS: usize = 100_000;

/// Group filtered rows by a typed key and evaluate a numeric aggregate on each group.
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
    let group_type = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .map(|candidate| candidate.data_type.as_str())
        .ok_or_else(|| ExecutionError::Invalid("unknown group column".into()))?;
    writeln!(output, "{column}\t{}({value_column})", function.name())?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let mut reader = TypedQueryReader::open(part, &names)?;

    let mut by_key = HashMap::<typed::Key, usize>::new();
    // Keep String keys in an owned map and probe with borrowed bytes.
    // Repeated station names no longer allocate a new key on every row.
    let mut text_groups = HashMap::<Vec<u8>, usize>::new();
    let mut groups = Vec::<(Cell, AggregateState)>::new();
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let borrowed_text = match &batch[group_index] {
                TypedColumn::Text(strings) => strings.get(row),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => values.get(row),
                _ => None,
            };
            let group = if let Some(key) = borrowed_text {
                if let Some(&existing) = text_groups.get(key) {
                    existing
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 100000 groups".into(),
                        ));
                    }
                    let index = groups.len();
                    text_groups.insert(key.to_vec(), index);
                    groups.push((Cell::Text(key.to_vec()), AggregateState::new(*function)));
                    index
                }
            } else {
                let key = typed::cell(&batch[group_index], row);
                match by_key.entry(key.key()) {
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
                }
            };
            let value = typed::cell(&batch[value_index], row);
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
    if let Some(having) = &query.having {
        groups.retain(|(_, state)| state.passes_having(having));
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
        typed::write_typed_value(output, &key, group_type)?;
        writeln!(output, "\t{}", state.render())?;
    }
    Ok(())
}
