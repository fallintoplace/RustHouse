//! Numeric aggregates grouped by multiple typed columns.
use super::typed::{self, Cell, Key};
use super::{AggregateState, ExecutionError, Number};
use crate::{AggregateFunction, Select};
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;

// Convert a non-nullable 8-bit numeric key into a compact slot index.
fn byte_group_key(column: &TypedColumn, row: usize) -> Option<u8> {
    match column {
        TypedColumn::Numeric(NumericColumn::UInt8(values)) => values.get(row).copied(),
        TypedColumn::Numeric(NumericColumn::Int8(values)) => {
            values.get(row).map(|value| *value as u8)
        }
        _ => None,
    }
}

fn numeric_value_at(column: &TypedColumn, row: usize) -> Result<Option<Number>, ExecutionError> {
    // Avoid constructing a generic Cell for every aggregate value in
    // large GROUP BY scans. NULLs are filtered directly from the null map.
    match column {
        TypedColumn::Numeric(values) => Ok(Some(super::number_at(values, row))),
        TypedColumn::NullableNumeric { values, nulls } => {
            if nulls[row] != 0 {
                Ok(None)
            } else {
                Ok(Some(super::number_at(values, row)))
            }
        }
        _ => Err(ExecutionError::Invalid(
            "numeric aggregates require numeric columns".into(),
        )),
    }
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    columns: &[String],
    items: &[(AggregateFunction, String)],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    for column in columns {
        if !types.contains_key(column) {
            return Err(ExecutionError::Invalid(format!(
                "unknown group column: {column}"
            )));
        }
    }

    let mut names = columns.to_vec();
    for (function, column) in items {
        let kind = types.get(column).ok_or_else(|| {
            ExecutionError::Invalid(format!("unknown aggregate column: {column}"))
        })?;
        if *function != AggregateFunction::Count
            && matches!(typed::base_type(kind), "String" | "UUID")
        {
            return Err(ExecutionError::Invalid(
                "numeric aggregates require numeric columns".into(),
            ));
        }
        if !names.contains(column) {
            names.push(column.clone());
        }
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let column = predicate.column();
        if !names.iter().any(|name| name == column) {
            names.push(column.to_owned());
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }

    for (index, column) in columns.iter().enumerate() {
        if index != 0 {
            output.write_all(b"\t")?;
        }
        output.write_all(column.as_bytes())?;
    }
    for (function, column) in items {
        write!(output, "\t{}({column})", function.name())?;
    }
    writeln!(output)?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let group_indices = columns
        .iter()
        .map(|column| indices[column])
        .collect::<Vec<_>>();
    let item_indices = items
        .iter()
        .map(|(_, column)| indices[column])
        .collect::<Vec<_>>();
    let mut by_key = HashMap::<Vec<Key>, usize>::new();
    let mut groups = Vec::<(Vec<Cell>, Vec<AggregateState>)>::new();
    let mut dense_slots = (columns.len() == 2
        && columns
            .iter()
            .all(|key| matches!(types.get(key).map(String::as_str), Some("UInt8" | "Int8"))))
    .then(|| vec![usize::MAX; 1 << 16]);
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    // Reuse a probe buffer for existing groups instead of allocating a Vec
    // of Cells and a Vec of Keys for every matching input row.
    let mut probe_key = Vec::<Key>::with_capacity(group_indices.len());
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let group = if let Some(slots) = dense_slots.as_mut() {
                let first = byte_group_key(&batch[group_indices[0]], row).ok_or_else(|| {
                    ExecutionError::Invalid("unexpected first byte GROUP BY type".into())
                })?;
                let second = byte_group_key(&batch[group_indices[1]], row).ok_or_else(|| {
                    ExecutionError::Invalid("unexpected second byte GROUP BY type".into())
                })?;
                let slot = (usize::from(first) << 8) | usize::from(second);
                let index = &mut slots[slot];
                if *index != usize::MAX {
                    *index
                } else {
                    *index = groups.len();
                    groups.push((
                        group_indices
                            .iter()
                            .map(|&column| typed::cell(&batch[column], row))
                            .collect(),
                        items
                            .iter()
                            .map(|(function, _)| AggregateState::new(*function))
                            .collect(),
                    ));
                    *index
                }
            } else {
                probe_key.clear();
                for &index in &group_indices {
                    probe_key.push(typed::key_at(&batch[index], row));
                }
                if let Some(&index) = by_key.get(probe_key.as_slice()) {
                    index
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 200000 groups".into(),
                        ));
                    }
                    let index = groups.len();
                    let keys = group_indices
                        .iter()
                        .map(|&column| typed::cell(&batch[column], row))
                        .collect::<Vec<_>>();
                    by_key.insert(std::mem::take(&mut probe_key), index);
                    probe_key = Vec::with_capacity(group_indices.len());
                    groups.push((
                        keys,
                        items
                            .iter()
                            .map(|(function, _)| AggregateState::new(*function))
                            .collect(),
                    ));
                    index
                }
            };
            for ((function, _), (&index, state)) in items
                .iter()
                .zip(item_indices.iter().zip(groups[group].1.iter_mut()))
            {
                if *function == AggregateFunction::Count {
                    if !typed::is_null_at(&batch[index], row) {
                        state.update(Number::Unsigned(1))?;
                    }
                    continue;
                }
                if let Some(value) = numeric_value_at(&batch[index], row)? {
                    state.update(value)?;
                }
            }
        }
    }

    let terms = query
        .order_terms()
        .map(|term| {
            let index = columns
                .iter()
                .position(|column| column == &term.column)
                .expect("GROUP BY ORDER BY validated by parser");
            (index, term.descending, term.nulls_first)
        })
        .collect::<Vec<_>>();
    let compare = |(left, _): &(Vec<Cell>, Vec<AggregateState>),
                   (right, _): &(Vec<Cell>, Vec<AggregateState>)| {
        for &(index, descending, nulls_first) in &terms {
            let order =
                typed::compare_order_cells(&left[index], &right[index], descending, nulls_first);
            if order != Ordering::Equal {
                return order;
            }
        }
        Ordering::Equal
    };
    let mut bounded_order = None;
    if !terms.is_empty() {
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
        |keys: &[Cell], states: &[AggregateState], output: &mut W| -> Result<(), ExecutionError> {
            for (index, key) in keys.iter().enumerate() {
                if index != 0 {
                    output.write_all(b"\t")?;
                }
                typed::write_typed_value(output, key, &types[&columns[index]])?;
            }
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
        for (keys, states) in groups
            .into_iter()
            .skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX))
        {
            write_group(&keys, &states, output)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod direct_grouped_numeric_tests {
    use super::*;

    #[test]
    fn reads_grouped_aggregate_values_and_skips_nulls() {
        let plain = TypedColumn::Numeric(NumericColumn::UInt64(vec![7, 11]));
        assert!(matches!(
            numeric_value_at(&plain, 1).unwrap(),
            Some(Number::Unsigned(11))
        ));
        let nullable = TypedColumn::NullableNumeric {
            values: NumericColumn::Int16(vec![-8, 99]),
            nulls: vec![0, 1],
        };
        assert!(matches!(
            numeric_value_at(&nullable, 0).unwrap(),
            Some(Number::Signed(-8))
        ));
        assert!(numeric_value_at(&nullable, 1).unwrap().is_none());
    }
}
