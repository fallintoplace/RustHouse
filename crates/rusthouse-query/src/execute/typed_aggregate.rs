//! Numeric aggregates over typed wide parts, including text filters and nullable values.
use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError, Number};
use crate::{AggregateFunction, Projection, Select};
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, hash_map::Entry};
use std::io::Write;
use std::path::Path;

fn replaces_text_extreme(
    value: &[u8],
    current: Option<&Cell>,
    function: AggregateFunction,
) -> bool {
    let Some(Cell::Text(best)) = current else {
        return current.is_none();
    };
    match function {
        AggregateFunction::Min => value < best.as_slice(),
        AggregateFunction::Max => value > best.as_slice(),
        _ => false,
    }
}

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
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();

    let mut state = AggregateState::new(*function);
    let mut ordered_value: Option<Cell> = None;
    // Only stringify the winning UUID, not every scanned UUID row.
    let mut ordered_uuid: Option<[u8; 16]> = None;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            if ordered_text {
                match &batch[projected_index] {
                    TypedColumn::Text(strings)
                    | TypedColumn::NullableText {
                        values: strings, ..
                    } => {
                        if typed::is_null_at(&batch[projected_index], row) {
                            continue;
                        }
                        let value = strings.get(row).ok_or_else(|| {
                            ExecutionError::Invalid("missing String value in aggregate".into())
                        })?;
                        if replaces_text_extreme(value, ordered_value.as_ref(), *function) {
                            ordered_value = Some(Cell::Text(value.to_vec()));
                        }
                        continue;
                    }
                    TypedColumn::Uuid(values) => {
                        let value = *values[row].as_bytes();
                        let replace = ordered_uuid.is_none_or(|best| match function {
                            AggregateFunction::Min => value < best,
                            AggregateFunction::Max => value > best,
                            _ => false,
                        });
                        if replace {
                            ordered_uuid = Some(value);
                            ordered_value = Some(Cell::Uuid(values[row].to_string()));
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            // Avoid constructing Cell enums on the common numeric batch path.
            // NULLs remain skipped, and checked AggregateState math is shared.
            match &batch[projected_index] {
                TypedColumn::Numeric(values) => {
                    state.update(super::number_at(values, row))?;
                    continue;
                }
                TypedColumn::NullableNumeric { values, nulls } => {
                    if nulls[row] == 0 {
                        state.update(super::number_at(values, row))?;
                    }
                    continue;
                }
                _ => {}
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

/// Group filtered rows by a typed key and evaluate a numeric aggregate.
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
    if *function != AggregateFunction::Count
        && matches!(typed::base_type(&selected.data_type), "String" | "UUID")
    {
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
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;

    let mut by_key = HashMap::<typed::Key, usize>::new();
    // Keep String keys in an owned map and probe with borrowed bytes.
    // Repeated station names no longer allocate a new key on every row.
    let mut text_groups = HashMap::<Vec<u8>, usize>::new();
    let mut uuid_groups = HashMap::<[u8; 16], usize>::new();
    let mut groups = Vec::<(Cell, AggregateState)>::new();
    // Non-nullable byte keys can be indexed directly rather than hashed.
    let mut dense_slots = match group_type {
        "UInt8" | "Int8" => Some(vec![usize::MAX; 256]),
        "UInt16" | "Int16" => Some(vec![usize::MAX; 65_536]),
        _ => None,
    };
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let group = if let Some(slots) = dense_slots.as_mut() {
                let key = match &batch[group_index] {
                    TypedColumn::Numeric(NumericColumn::UInt8(values)) => usize::from(values[row]),
                    TypedColumn::Numeric(NumericColumn::Int8(values)) => {
                        usize::from(values[row] as u8)
                    }
                    TypedColumn::Numeric(NumericColumn::UInt16(values)) => usize::from(values[row]),
                    TypedColumn::Numeric(NumericColumn::Int16(values)) => {
                        usize::from(values[row] as u16)
                    }
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected dense GROUP BY column".into(),
                        ));
                    }
                };
                let group = &mut slots[key];
                if *group != usize::MAX {
                    *group
                } else {
                    let index = groups.len();
                    *group = index;
                    groups.push((
                        typed::cell(&batch[group_index], row),
                        AggregateState::new(*function),
                    ));
                    index
                }
            } else {
                let borrowed_text = match &batch[group_index] {
                    TypedColumn::Text(strings) => strings.get(row),
                    TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
                        values.get(row)
                    }
                    _ => None,
                };
                let borrowed_uuid = match &batch[group_index] {
                    TypedColumn::Uuid(values) => Some(*values[row].as_bytes()),
                    _ => None,
                };
                if let Some(key) = borrowed_uuid {
                    if let Some(&existing) = uuid_groups.get(&key) {
                        existing
                    } else {
                        if groups.len() >= MAX_GROUPS {
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 100000 groups".into(),
                            ));
                        }
                        let index = groups.len();
                        uuid_groups.insert(key, index);
                        groups.push((
                            typed::cell(&batch[group_index], row),
                            AggregateState::new(*function),
                        ));
                        index
                    }
                } else if let Some(key) = borrowed_text {
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
                }
            };
            if *function == AggregateFunction::Count {
                if !typed::is_null_at(&batch[value_index], row) {
                    groups[group].1.increment_count()?;
                }
                continue;
            }
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
    let mut bounded_order = None;
    if query.order_by.is_some() {
        let compare = |(left_key, left_state): &(Cell, AggregateState),
                       (right_key, right_state): &(Cell, AggregateState)| {
            for term in query.order_terms() {
                let comparison = if term.column == "#aggregate" {
                    typed::compare_order_cells(
                        &left_state.sort_cell(),
                        &right_state.sort_cell(),
                        term.descending,
                        term.nulls_first,
                    )
                } else {
                    typed::compare_order_cells(
                        left_key,
                        right_key,
                        term.descending,
                        term.nulls_first,
                    )
                };
                if comparison != std::cmp::Ordering::Equal {
                    return comparison;
                }
            }
            std::cmp::Ordering::Equal
        };
        let end = query
            .limit
            .and_then(|limit| query.offset.unwrap_or(0).checked_add(limit));
        if let Some(end) = end.filter(|&end| end < groups.len()) {
            let mut indices = (0..groups.len()).collect::<Vec<_>>();
            let compare_index = |left: &usize, right: &usize| {
                compare(&groups[*left], &groups[*right]).then_with(|| left.cmp(right))
            };
            indices.select_nth_unstable_by(end, &compare_index);
            indices.truncate(end);
            indices.sort_unstable_by(compare_index);
            bounded_order = Some(indices);
        } else {
            groups.sort_by(compare);
        }
    }
    let write_group =
        |key: &Cell, state: &AggregateState, output: &mut W| -> Result<(), ExecutionError> {
            typed::write_typed_value(output, key, group_type)?;
            writeln!(output, "\t{}", state.render())?;
            Ok(())
        };
    if let Some(indices) = bounded_order {
        for index in indices.into_iter().skip(query.offset.unwrap_or(0)) {
            write_group(&groups[index].0, &groups[index].1, output)?;
        }
    } else {
        for (key, state) in groups
            .iter()
            .skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX))
        {
            write_group(key, state, output)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod borrowed_text_extrema_tests {
    use super::*;

    #[test]
    fn compares_borrowed_text_without_replacing_ties() {
        let incumbent = Cell::Text(b"beta".to_vec());
        assert!(replaces_text_extreme(
            b"alpha",
            Some(&incumbent),
            AggregateFunction::Min
        ));
        assert!(!replaces_text_extreme(
            b"beta",
            Some(&incumbent),
            AggregateFunction::Min
        ));
        assert!(!replaces_text_extreme(
            b"alpha",
            Some(&incumbent),
            AggregateFunction::Max
        ));
        assert!(replaces_text_extreme(
            b"gamma",
            Some(&incumbent),
            AggregateFunction::Max
        ));
        assert!(replaces_text_extreme(b"beta", None, AggregateFunction::Min));
    }
}
