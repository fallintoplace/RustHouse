use super::ExecutionError;
use super::typed::{self, Cell, Key};
use crate::Select;
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;
const MAX_PARALLEL_KEY_BYTES: usize = 64 * 1024 * 1024;

fn reserve_parallel_group(
    budget: &std::sync::atomic::AtomicUsize,
    keys: &[Key],
) -> Result<(), ExecutionError> {
    use std::sync::atomic::Ordering;
    let fixed = keys.len().checked_mul(
        std::mem::size_of::<Key>() + std::mem::size_of::<Cell>()
    ).ok_or_else(|| ExecutionError::Invalid("GROUP BY key accounting overflow".into()))?;
    let bytes = keys.iter().try_fold(fixed, |sum, key| {
        let extra = match key {
            Key::Text(text) => text.len().checked_mul(2),
            Key::Uuid(value) => value.len().checked_mul(2),
            _ => Some(0),
        }.ok_or_else(|| ExecutionError::Invalid("GROUP BY key accounting overflow".into()))?;
        sum.checked_add(extra)
            .ok_or_else(|| ExecutionError::Invalid("GROUP BY key accounting overflow".into()))
    })?;
    budget.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        current.checked_add(bytes).filter(|&total| total <= MAX_PARALLEL_KEY_BYTES)
    }).map(|_| ()).map_err(|_| ExecutionError::Invalid(
        "parallel GROUP BY exceeds 64 MiB worker key budget".into()
    ))
}


const MAX_GROUP_KEY_BYTES: usize = 64 * 1024 * 1024;

fn charge_group_memory(
    used: &mut usize,
    dimensions: usize,
    encoded_bytes: usize,
) -> Result<(), ExecutionError> {
    let bytes = dimensions.checked_mul(
        std::mem::size_of::<Key>() + std::mem::size_of::<Cell>()
    ).and_then(|fixed| encoded_bytes.checked_mul(2)
        .and_then(|variable| fixed.checked_add(variable)))
        .and_then(|total| used.checked_add(total))
        .ok_or_else(|| ExecutionError::Invalid("group key memory overflow".into()))?;
    if bytes > MAX_GROUP_KEY_BYTES {
        return Err(ExecutionError::Invalid("GROUP BY exceeds 64 MiB retained key budget".into()));
    }
    *used = bytes;
    Ok(())
}


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


fn parallel_grouped_parts(
    part: &Path,
    names: &[String],
    group_indices: &[usize],
    conditions: &[Vec<typed::Condition>],
    no_filters: bool,
) -> Result<Option<Vec<(Vec<Cell>, u64)>>, ExecutionError> {
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    // Static chunks strand workers behind one unusually large MergeTree part.
    // Assign whole parts dynamically while each worker keeps a private hash table.
    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let retained = std::sync::atomic::AtomicUsize::new(0);
    let chunks = std::thread::scope(|scope| {
        let retained = &retained;
        let cursor = &cursor;
        let paths = &parts;
        let handles = (0..workers).map(|_| {
            scope.spawn(move || -> Result<Vec<(Vec<Cell>, u64)>, ExecutionError> {
                let mut rows: Vec<(Vec<Cell>, u64)> = Vec::new();
                let mut index: HashMap<Vec<Key>, usize> = HashMap::new();
                // Frequent dimension keys should reuse their probe allocation
                // instead of allocating a Vec for every fact row.
                let mut probe = Vec::<Key>::with_capacity(group_indices.len());
                while let Some(path) = paths.get(
                    cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ) {
                    let mut reader = rusthouse_storage::TypedPartReader::open(path, names)?;
                    while let Some(batch) = reader.next_batch()? {
                        for row in 0..batch[0].len() {
                            if !no_filters && !typed::passes(&batch, row, conditions) {
                                continue;
                            }
                            probe.clear();
                            probe.extend(group_indices.iter()
                                .map(|&column| typed::key_at(&batch[column], row)));
                            if let Some(&found) = index.get(probe.as_slice()) {
                                rows[found].1 = rows[found].1.checked_add(1)
                                    .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
                            } else {
                                if rows.len() >= MAX_GROUPS {
                                    return Err(ExecutionError::Invalid(
                                        "GROUP BY supports at most 200000 groups".into(),
                                    ));
                                }
                                reserve_parallel_group(retained, &probe)?;
                                index.insert(std::mem::take(&mut probe), rows.len());
                                probe = Vec::with_capacity(group_indices.len());
                                rows.push((
                                    group_indices.iter()
                                        .map(|&column| typed::cell(&batch[column], row))
                                        .collect(),
                                    1,
                                ));
                            }
                        }
                    }
                }
                Ok(rows)
            })
        }).collect::<Vec<_>>();
        handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| {
            Err(ExecutionError::Invalid("parallel grouped worker panicked".into()))
        })).collect::<Vec<_>>()
    });
    let mut rows = Vec::<(Vec<Cell>, u64)>::new();
    let mut by_key = HashMap::<Vec<Key>, usize>::new();
    for chunk in chunks {
        for (key_values, count) in chunk? {
            let key = key_values.iter().map(Cell::key).collect::<Vec<_>>();
            if let Some(&found) = by_key.get(&key) {
                rows[found].1 = rows[found].1.checked_add(count)
                    .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
            } else {
                if rows.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 200000 groups".into(),
                    ));
                }
                by_key.insert(key, rows.len());
                rows.push((key_values, count));
            }
        }
    }
    Ok(Some(rows))
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
    let partial = parallel_grouped_parts(
        part,
        &names,
        &group_indices,
        &conditions,
        query.predicates.is_empty() && query.or_predicates.is_empty(),
    )?;
    // Some(empty) is a completed parallel scan with zero matching rows.
    // It is not a request to decode and rescan the full snapshot serially.
    let mut reader = if partial.is_none() {
        Some(TypedQueryReader::open_with_metadata(part, &names, &metadata)?)
    } else {
        None
    };
    let mut groups: Vec<(Vec<Cell>, u64)> = partial.unwrap_or_default();
    let mut retained_group_bytes = 0usize;
    // A parallel scan can return groups without entering the serial loop.
    for (keys, _) in &groups {
        let payload = keys.iter().try_fold(0usize, |total, cell| {
            let variable = match cell {
                Cell::Text(bytes) => bytes.len(),
                Cell::Uuid(text) => text.len(),
                _ => 0,
            };
            total.checked_add(variable)
                .ok_or_else(|| ExecutionError::Invalid("group key memory overflow".into()))
        })?;
        charge_group_memory(&mut retained_group_bytes, keys.len(), payload)?;
    }
    let mut by_key: HashMap<Vec<Key>, usize> = HashMap::new();
    let mut probe_key = Vec::with_capacity(group_indices.len());
    // Probe repeated String dimensions without cloning their decoded bytes.
    let has_text_key = columns
        .iter()
        .any(|column| matches!(typed::base_type(types[column].as_str()), "String"));
    let mut encoded_groups = HashMap::<Vec<u8>, usize>::new();
    let mut encoded_probe = Vec::new();
    let mut dense_groups = (columns.len() == 2
        && columns
            .iter()
            .all(|name| matches!(types.get(name).map(String::as_str), Some("UInt8" | "Int8"))))
    .then(|| vec![usize::MAX; 1 << 16]);
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.as_mut().map_or(Ok(None), TypedQueryReader::next_batch)? {
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
                    charge_group_memory(&mut retained_group_bytes, group_indices.len(), 0)?;
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
            if has_text_key {
                encoded_probe.clear();
                for &index in &group_indices {
                    super::grouped_numeric_keys::append_group_component(
                        &batch[index], row, &mut encoded_probe
                    );
                }
                if let Some(&index) = encoded_groups.get(encoded_probe.as_slice()) {
                    groups[index].1 = groups[index]
                        .1
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
                    charge_group_memory(
                        &mut retained_group_bytes, group_indices.len(), encoded_probe.len(),
                    )?;
                    encoded_groups.insert(std::mem::take(&mut encoded_probe), groups.len());
                    encoded_probe = Vec::with_capacity(group_indices.len() * 24);
                    groups.push((values, 1));
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
                charge_group_memory(&mut retained_group_bytes, group_indices.len(), 0)?;
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

#[cfg(test)]
mod grouped_count_memory_budget_tests {
    use super::*;
    #[test]
    fn key_budget_rejects_wide_tuples_without_mutating_accounting() {
        let mut allocated = 0usize;
        charge_group_memory(&mut allocated, 8, 256).unwrap();
        let before = allocated;
        assert!(charge_group_memory(&mut allocated, 8, MAX_GROUP_KEY_BYTES).is_err());
        assert_eq!(allocated, before);
        assert!(charge_group_memory(&mut allocated, usize::MAX, 0).is_err());
    }
}

#[cfg(test)]
mod parallel_key_budget_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn concurrent_group_reservations_never_exceed_global_budget() {
        let used = AtomicUsize::new(0);
        let huge_key = Key::Text(vec![0; 1024 * 1024]);
        let group = [huge_key];
        let count = std::thread::scope(|scope| {
            let handles = (0..64).map(|_| scope.spawn(|| reserve_parallel_group(&used, &group).is_ok()))
                .collect::<Vec<_>>();
            handles.into_iter().filter(|task| task.join().unwrap()).count()
        });
        assert!(count > 0 && count < 64);
        assert!(used.load(Ordering::Relaxed) <= MAX_PARALLEL_KEY_BYTES);
    }
}
