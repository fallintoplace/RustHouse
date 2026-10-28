use super::ExecutionError;
use super::typed::{self, Cell, Key};
use crate::Select;
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;

// Reinterpret signed 8-bit keys as indices, retaining the original signed
// values in result cells. A two-byte tuple needs at most 65,536 slots.
fn byte_group_key(column: &TypedColumn, row: usize) -> Option<u8> {
    match column {
        TypedColumn::Numeric(NumericColumn::UInt8(values)) => values.get(row).copied(),
        TypedColumn::Numeric(NumericColumn::Int8(values)) => {
            values.get(row).map(|value| *value as u8)
        }
        _ => None,
    }
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    columns: &[String],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut names = columns.to_vec();
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let name = predicate.column();
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_owned());
        }
    }
    let indices: HashMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let types: HashMap<_, _> = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect();
    for name in &names {
        if !types.contains_key(name) {
            return Err(ExecutionError::Invalid(format!("unknown column: {name}")));
        }
    }
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
    writeln!(output, "\t{}", query.alias.as_deref().unwrap_or("count"))?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let group_indices = columns.iter().map(|name| indices[name]).collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let mut groups: Vec<(Vec<Cell>, u64)> = Vec::new();
    let mut by_key: HashMap<Vec<Key>, usize> = HashMap::new();
    let mut probe_key = Vec::with_capacity(group_indices.len());
    let mut dense_groups = (columns.len() == 2
        && columns
            .iter()
            .all(|name| matches!(types.get(name).map(String::as_str), Some("UInt8" | "Int8"))))
    .then(|| vec![usize::MAX; 1 << 16]);
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            if let Some(by_pair) = dense_groups.as_mut() {
                let left = byte_group_key(&batch[group_indices[0]], row).ok_or_else(|| {
                    ExecutionError::Invalid("unexpected first byte GROUP BY type".into())
                })?;
                let right = byte_group_key(&batch[group_indices[1]], row).ok_or_else(|| {
                    ExecutionError::Invalid("unexpected second byte GROUP BY type".into())
                })?;
                let slot = (usize::from(left) << 8) | usize::from(right);
                let index = &mut by_pair[slot];
                if *index != usize::MAX {
                    groups[*index].1 = groups[*index]
                        .1
                        .checked_add(1)
                        .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
                } else {
                    *index = groups.len();
                    groups.push((
                        group_indices
                            .iter()
                            .map(|&index| typed::cell(&batch[index], row))
                            .collect(),
                        1,
                    ));
                }
                continue;
            }
            // Build only the probe key for existing groups. Wide projection
            // cells are materialized once, when a new group is discovered.
            probe_key.clear();
            for &column in &group_indices {
                probe_key.push(typed::key_at(&batch[column], row));
            }
            if let Some(&index) = by_key.get(probe_key.as_slice()) {
                let count = &mut groups[index].1;
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 200000 groups".into(),
                    ));
                }
                let values: Vec<Cell> = group_indices
                    .iter()
                    .map(|&index| typed::cell(&batch[index], row))
                    .collect();
                by_key.insert(std::mem::take(&mut probe_key), groups.len());
                probe_key = Vec::with_capacity(group_indices.len());
                groups.push((values, 1));
            }
        }
    }
    if let Some(having) = &query.having {
        groups.retain(|(_, count)| {
            super::predicates::number_matches(
                super::Number::Unsigned(u128::from(*count)),
                super::Number::Unsigned(u128::from(having.value)),
                having.operator,
            )
        });
    }
    let mut bounded_order = None;
    if query.order_by.is_some() {
        let sort_terms = query
            .order_terms()
            .map(|order| {
                let index = (order.column != "count").then(|| {
                    columns
                        .iter()
                        .position(|column| column == &order.column)
                        .expect("ORDER BY was checked during parsing")
                });
                (index, order.descending, order.nulls_first)
            })
            .collect::<Vec<_>>();
        let compare = |(left_keys, left_count): &(Vec<Cell>, u64),
                       (right_keys, right_count): &(Vec<Cell>, u64)| {
            for &(index, descending, nulls_first) in &sort_terms {
                let cmp = if let Some(index) = index {
                    typed::compare_order_cells(
                        &left_keys[index],
                        &right_keys[index],
                        descending,
                        nulls_first,
                    )
                } else {
                    let comparison = left_count.cmp(right_count);
                    if descending {
                        comparison.reverse()
                    } else {
                        comparison
                    }
                };
                if cmp != Ordering::Equal {
                    return cmp;
                }
            }
            Ordering::Equal
        };
        let end = query
            .limit
            .and_then(|limit| query.offset.unwrap_or(0).checked_add(limit));
        if let Some(end) = end.filter(|&end| end < groups.len()) {
            let mut indices = (0..groups.len()).collect::<Vec<_>>();
            // The original index preserves stable sort behavior on ties.
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
    // Resolve output schema once before serializing many grouped rows.
    let output_types = columns
        .iter()
        .map(|column| types[column].as_str())
        .collect::<Vec<_>>();
    let write_group = |keys: &[Cell], count: u64, output: &mut W| -> Result<(), ExecutionError> {
        for (index, value) in keys.iter().enumerate() {
            if index != 0 {
                output.write_all(b"\t")?;
            }
            typed::write_typed_value(output, value, output_types[index])?;
        }
        writeln!(output, "\t{count}")?;
        Ok(())
    };
    if let Some(indices) = bounded_order {
        for index in indices.into_iter().skip(query.offset.unwrap_or(0)) {
            write_group(&groups[index].0, groups[index].1, output)?;
        }
    } else {
        for (keys, count) in groups
            .into_iter()
            .skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX))
        {
            write_group(&keys, count, output)?;
        }
    }
    Ok(())
}
