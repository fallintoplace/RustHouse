//! Multiple grouped numeric aggregates over one typed scan.
use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError, Number};
use crate::{AggregateFunction, Select};
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;

fn group_limit_error() -> ExecutionError {
    ExecutionError::Invalid("GROUP BY supports at most 200000 groups".into())
}

fn numeric_column_error() -> ExecutionError {
    ExecutionError::Invalid("numeric aggregates require numeric columns".into())
}

fn filter_enabled(column: &TypedColumn, row: usize) -> Result<bool, ExecutionError> {
    let number = match column {
        TypedColumn::Numeric(values) => Some(super::number_at(values, row)),
        TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 => {
            Some(super::number_at(values, row))
        }
        TypedColumn::NullableNumeric { .. } => None,
        _ => return Err(ExecutionError::Invalid(
            "conditional metric filter must be numeric".into(),
        )),
    };
    Ok(match number {
        Some(Number::Unsigned(value)) => value != 0,
        Some(Number::Signed(value)) => value != 0,
        Some(Number::Float(value)) => value != 0.0,
        None => false,
    })
}


const MAX_GROUP_STATE_BYTES: usize = 64 * 1024 * 1024;

fn reserve_median_value(retained: &mut usize) -> Result<(), ExecutionError> {
    let bytes = retained.checked_add(std::mem::size_of::<Number>())
        .ok_or_else(|| ExecutionError::Invalid("grouped median memory overflow".into()))?;
    if bytes > MAX_GROUP_STATE_BYTES {
        return Err(ExecutionError::Invalid(
            "grouped median values exceed 64 MiB retained budget".into(),
        ));
    }
    *retained = bytes;
    Ok(())
}

fn reserve_group(
    retained: &mut usize,
    metric_count: usize,
    variable_bytes: usize,
) -> Result<(), ExecutionError> {
    let fixed = std::mem::size_of::<(Cell, Vec<AggregateState>)>()
        .checked_add(std::mem::size_of::<typed::Key>())
        .and_then(|v| metric_count.checked_mul(std::mem::size_of::<AggregateState>())
            .and_then(|states| v.checked_add(states)))
        .ok_or_else(|| ExecutionError::Invalid("group memory estimate overflow".into()))?;
    let next = variable_bytes.checked_mul(2)
        .and_then(|variable| fixed.checked_add(variable))
        .and_then(|additional| retained.checked_add(additional))
        .ok_or_else(|| ExecutionError::Invalid("group memory estimate overflow".into()))?;
    if next > MAX_GROUP_STATE_BYTES {
        return Err(ExecutionError::Invalid("GROUP BY exceeds 64 MiB group-state budget".into()));
    }
    *retained = next;
    Ok(())
}


pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    items: &[(AggregateFunction, String)],
    aliases: &[Option<String>],
    filters: &[Option<String>],
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
            return Err(numeric_column_error());
        }
    }
    for filter in filters.iter().flatten() {
        if !names.contains(filter) {
            names.push(filter.clone());
        }
        let column_type = metadata.columns.iter().find(|column| &column.name == filter)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown filter column: {filter}")))?;
        if matches!(typed::base_type(&column_type.data_type), "String" | "UUID") {
            return Err(ExecutionError::Invalid("conditional metric filters must be numeric".into()));
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
    for (index, (function, value_column)) in items.iter().enumerate() {
        if let Some(alias) = aliases.get(index).and_then(Option::as_deref) {
            write!(output, "\t{alias}")?;
        } else {
            if let Some(filter) = filters.get(index).and_then(Option::as_deref) {
                write!(output, "\t{}If({value_column}, {filter})", function.name())?;
            } else {
            write!(output, "\t{}({value_column})", function.name())?;
            }
        }
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
    let filter_indices = filters.iter()
        .map(|value| value.as_ref().map(|column| indices[column]))
        .collect::<Vec<_>>();
    let mut groups = Vec::<(Cell, Vec<AggregateState>)>::new();
    let mut retained_group_bytes = 0usize;
    let mut retained_median_bytes = 0usize;
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
                    reserve_group(&mut retained_group_bytes, items.len(), 0)?;
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
                            return Err(group_limit_error());
                        }
                        reserve_group(&mut retained_group_bytes, items.len(), 0)?;
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
                            return Err(group_limit_error());
                        }
                        reserve_group(&mut retained_group_bytes, items.len(), key.len())?;
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
                                return Err(group_limit_error());
                            }
                            reserve_group(&mut retained_group_bytes, items.len(), 0)?;
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
            for (metric, ((function, _), (&column_index, state))) in items
                .iter()
                .zip(item_indices.iter().zip(groups[index].1.iter_mut()))
                .enumerate()
            {
                if let Some(&filter_index) = filter_indices.get(metric).and_then(Option::as_ref) {
                    let enabled = filter_enabled(&batch[filter_index], row)?;
                    if !enabled {continue;}
                }
                if *function == AggregateFunction::Median
                    && !typed::is_null_at(&batch[column_index], row)
                {
                    reserve_median_value(&mut retained_median_bytes)?;
                }
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
                            return Err(numeric_column_error());
                        }
                    },
                }
            }
        }
    }
    if let Some(having) = &query.having {
        let index = having.aggregate_index.ok_or_else(|| {
            ExecutionError::Invalid("HAVING metric index is required".into())
        })?;
        groups.retain(|(_, states)| states.get(index)
            .is_some_and(|state| state.passes_having(having)));
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
        let cached_medians = terms.iter().filter_map(|(metric, _, _)| *metric)
            .filter(|&metric| items[metric].0 == AggregateFunction::Median)
            .map(|metric| (metric, groups.iter()
                .map(|(_, states)| states[metric].sort_cell())
                .collect::<Vec<_>>()))
            .collect::<HashMap<_, _>>();
        if !cached_medians.is_empty() {
            // Avoid cloning a median's entire input vector in each
            // comparison, including multi-term ORDER BY and paged top-K.
            let mut indices = (0..groups.len()).collect::<Vec<_>>();
            let compare_cached = |left: &usize, right: &usize| {
                for &(metric, descending, nulls_first) in &terms {
                    let order = if let Some(index) = metric {
                        if let Some(keys) = cached_medians.get(&index) {
                            typed::compare_order_cells(
                                &keys[*left], &keys[*right], descending, nulls_first,
                            )
                        } else {
                            let left_cell = groups[*left].1[index].sort_cell();
                            let right_cell = groups[*right].1[index].sort_cell();
                            typed::compare_order_cells(
                                &left_cell, &right_cell, descending, nulls_first,
                            )
                        }
                    } else {
                        typed::compare_order_cells(&groups[*left].0, &groups[*right].0,
                            descending, nulls_first)
                    };
                    if order != std::cmp::Ordering::Equal { return order; }
                }
                left.cmp(right)
            };
            if let Some(end) = end.filter(|&end| end < indices.len()) {
                indices.select_nth_unstable_by(end, &compare_cached);
                indices.truncate(end);
            }
            indices.sort_unstable_by(compare_cached);
            bounded_order = Some(indices);
        } else if let Some(end) = end.filter(|&end| end < groups.len()) {
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

#[cfg(test)]
mod group_state_budget_tests {
    use super::*;

    #[test]
    fn bounded_multi_metric_group_memory_cannot_wrap_or_overrun() {
        let mut used = 0usize;
        reserve_group(&mut used, 4, 128).unwrap();
        let first = used;
        reserve_group(&mut used, 4, 128).unwrap();
        assert_eq!(used, first * 2);
        assert!(reserve_group(&mut used, usize::MAX, 0).is_err());
        assert_eq!(used, first * 2);
        assert!(reserve_group(&mut used, 1, MAX_GROUP_STATE_BYTES).is_err());
    }
}

#[cfg(test)]
mod borrowed_metric_filter_tests {
    use super::*;
    #[test]
    fn handles_all_numeric_filter_types_without_materializing_cells() {
        let unsigned = TypedColumn::Numeric(NumericColumn::UInt8(vec![0, 1]));
        assert!(!filter_enabled(&unsigned, 0).unwrap());
        assert!(filter_enabled(&unsigned, 1).unwrap());
        let signed = TypedColumn::Numeric(NumericColumn::Int64(vec![-4, 0]));
        assert!(filter_enabled(&signed, 0).unwrap());
        assert!(!filter_enabled(&signed, 1).unwrap());
        let nullable = TypedColumn::NullableNumeric {
            values: NumericColumn::UInt64(vec![99, 0]),
            nulls: vec![1, 0],
        };
        assert!(!filter_enabled(&nullable, 0).unwrap());
        assert!(!filter_enabled(&nullable, 1).unwrap());
        let floats = TypedColumn::Numeric(NumericColumn::Float64(vec![0.0, 2.5]));
        assert!(!filter_enabled(&floats, 0).unwrap());
        assert!(filter_enabled(&floats, 1).unwrap());
    }
}

#[cfg(test)]
mod grouped_multi_median_budget_tests {
    use super::*;

    #[test]
    fn bounds_median_storage_across_metrics_and_groups() {
        let mut retained = 0;
        reserve_median_value(&mut retained).unwrap();
        let one = retained;
        reserve_median_value(&mut retained).unwrap();
        assert_eq!(retained, one * 2);
        retained = MAX_GROUP_STATE_BYTES - 1;
        assert!(reserve_median_value(&mut retained).is_err());
        assert_eq!(retained, MAX_GROUP_STATE_BYTES - 1);
    }
}

#[cfg(test)]
mod cached_multimetric_median_sort_tests {
    use super::*;
    #[test]
    fn borrowed_cached_medians_keep_same_sort_order() {
        let values = [3u128, 9, 1, 7];
        let keys = values.map(|value| {
            let mut median = AggregateState::new(AggregateFunction::Median);
            median.update(Number::Unsigned(value)).unwrap();
            median.sort_cell()
        });
        let mut positions = (0..keys.len()).collect::<Vec<_>>();
        positions.sort_unstable_by(|&a, &b| {
            typed::compare_order_cells(&keys[a], &keys[b], false, false)
        });
        assert_eq!(positions, [2, 0, 3, 1]);
    }

    #[test]
    fn cached_medians_preserve_multi_metric_order() {
        let groups = [(10u128, 4u128), (4, 2), (7, 3)].map(|(a, b)| {
            let mut state = AggregateState::new(AggregateFunction::Median);
            state.update(Number::Unsigned(a)).unwrap();
            state.update(Number::Unsigned(b)).unwrap();
            state
        });
        let cached = groups.iter().map(AggregateState::sort_cell).collect::<Vec<_>>();
        let mut order = (0..groups.len()).collect::<Vec<_>>();
        order.sort_by(|&left, &right| typed::compare_order_cells(
            &cached[left], &cached[right], false, None
        ));
        assert_eq!(order, [1, 2, 0]);
    }
}
