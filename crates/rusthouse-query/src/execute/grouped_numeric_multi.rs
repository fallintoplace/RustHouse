//! Multiple grouped numeric aggregates over one typed scan.
use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError, Number};
use crate::{AggregateFunction, Select};
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    items: &[(AggregateFunction, String)],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut names = vec![column.to_owned()];
    for (function, value_column) in items {
        if !names.contains(value_column) {
            names.push(value_column.clone());
        }
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
                "numeric aggregates require numeric columns".into(),
            ));
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
    let group_type = types
        .get(column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown group column: {column}")))?;
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

    let group_index = indices[column];
    let item_indices = items
        .iter()
        .map(|(_, name)| indices[name])
        .collect::<Vec<_>>();
    let mut groups = Vec::<(Cell, Vec<AggregateState>)>::new();
    // Direct slots avoid repeated hashing on compact grouping columns.
    let mut dense_slots = match group_type.as_str() {
        "UInt8" | "Int8" => Some(vec![usize::MAX; 256]),
        "UInt16" | "Int16" => Some(vec![usize::MAX; 65_536]),
        _ => None,
    };
    let mut by_key = HashMap::<typed::Key, usize>::new();
    let mut by_text = HashMap::<Vec<u8>, usize>::new();
    let mut by_uuid = HashMap::<[u8; 16], usize>::new();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            // Nullable String keys can use the same borrowed-byte lookup as String.
            // Only new groups need to own the decoded bytes.
            let index = if let Some(slots) = dense_slots.as_mut() {
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
                            "unexpected dense GROUP BY type".into(),
                        ));
                    }
                };
                let slot = &mut slots[key];
                if *slot != usize::MAX {
                    *slot
                } else {
                    let index = groups.len();
                    *slot = index;
                    groups.push((
                        typed::cell(&batch[group_index], row),
                        items
                            .iter()
                            .map(|(function, _)| AggregateState::new(*function))
                            .collect(),
                    ));
                    index
                }
            } else {
                let borrowed_text = match &batch[group_index] {
                    TypedColumn::Text(values) => values.get(row),
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
                    if let Some(&group) = by_uuid.get(&key) {
                        group
                    } else {
                        if groups.len() >= MAX_GROUPS {
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        let group = groups.len();
                        by_uuid.insert(key, group);
                        groups.push((
                            typed::cell(&batch[group_index], row),
                            items
                                .iter()
                                .map(|(function, _)| AggregateState::new(*function))
                                .collect(),
                        ));
                        group
                    }
                } else if let Some(key) = borrowed_text {
                    if let Some(&group) = by_text.get(key) {
                        group
                    } else {
                        if groups.len() >= MAX_GROUPS {
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        let group = groups.len();
                        by_text.insert(key.to_vec(), group);
                        groups.push((
                            Cell::Text(key.to_vec()),
                            items
                                .iter()
                                .map(|(function, _)| AggregateState::new(*function))
                                .collect(),
                        ));
                        group
                    }
                } else {
                    let key = typed::cell(&batch[group_index], row);
                    match by_key.entry(key.key()) {
                        Entry::Occupied(entry) => *entry.get(),
                        Entry::Vacant(entry) => {
                            if groups.len() >= MAX_GROUPS {
                                return Err(ExecutionError::Invalid(
                                    "GROUP BY supports at most 200000 groups".into(),
                                ));
                            }
                            entry.insert(groups.len());
                            groups.push((
                                key,
                                items
                                    .iter()
                                    .map(|(function, _)| AggregateState::new(*function))
                                    .collect(),
                            ));
                            groups.len() - 1
                        }
                    }
                }
            };
            for ((function, _), (&column_index, state)) in items
                .iter()
                .zip(item_indices.iter().zip(groups[index].1.iter_mut()))
            {
                if *function == AggregateFunction::Count {
                    if !typed::is_null_at(&batch[column_index], row) {
                        state.update(Number::Unsigned(1))?;
                    }
                    continue;
                }
                // Handle native numeric batches without creating a Cell
                // for each aggregate value and every matching input row.
                match &batch[column_index] {
                    TypedColumn::Numeric(values) => {
                        state.update(super::number_at(values, row))?;
                    }
                    TypedColumn::NullableNumeric { values, nulls } => {
                        if nulls[row] == 0 {
                            state.update(super::number_at(values, row))?;
                        }
                    }
                    column => match typed::cell(column, row) {
                        Cell::Numeric(value) => state.update(value)?,
                        Cell::Null => {}
                        Cell::Text(_) | Cell::Uuid(_) => {
                            return Err(ExecutionError::Invalid(
                                "numeric aggregates require numeric columns".into(),
                            ));
                        }
                    },
                }
            }
        }
    }
    let mut bounded_order = None;
    if query.order_by.is_some() {
        let terms = query.order_terms().map(|order| {
            let metric = order.column.strip_prefix("#aggregate")
                .and_then(|number| number.parse::<usize>().ok());
            (metric, order.descending, order.nulls_first)
        }).collect::<Vec<_>>();
        let compare = |left: &(Cell, Vec<AggregateState>), right: &(Cell, Vec<AggregateState>)| {
            for &(metric, descending, nulls_first) in &terms {
                let ordering = if let Some(index) = metric {
                    typed::compare_order_cells(&left.1[index].sort_cell(), &right.1[index].sort_cell(),
                        descending, nulls_first)
                } else {
                    typed::compare_order_cells(&left.0, &right.0, descending, nulls_first)
                };
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
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
        |key: &Cell, states: &[AggregateState], output: &mut W| -> Result<(), ExecutionError> {
            typed::write_typed_value(output, key, group_type)?;
            for state in states {
                write!(output, "\t{}", state.render())?;
            }
            writeln!(output)?;
            Ok(())
        };
    if let Some(indices) = bounded_order {
        for index in indices.into_iter().skip(query.offset.unwrap_or(0)) {
            write_group(&groups[index].0, &groups[index].1, output)?;
        }
    } else {
        for (key, states) in groups
            .into_iter()
            .skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX))
        {
            write_group(&key, &states, output)?;
        }
    }
    Ok(())
}
