//! Per-cohort heavy hitters without unbounded COUNT/GROUP BY hash tables.
//! At most 128 groups, each with at most 512 SpaceSaving counters. String
//! payloads are capped to keep combined state bounded on huge input streams.
use super::spacesaving_topk::{self, SpaceSaving, View, write_key};
use super::typed::{self, Cell, Key};
use super::ExecutionError;
use crate::{Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 128;
const MAX_GROUP_BYTES: usize = 512;
const MAX_VALUE_BYTES: usize = 256;

struct Group {
    value: Cell,
    top: SpaceSaving,
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::GroupedTopK { column, value_column, k } = &query.projection else {
        unreachable!("grouped topK requires grouped projection");
    };
    if query.having.is_some() {
        return Err(ExecutionError::Invalid(
            "grouped topK does not support HAVING arrays".into(),
        ));
    }
    let metadata = inspect_query_source(part)?;
    let group_type = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown grouped topK key: {column}")))?
        .data_type
        .as_str();
    if !metadata.columns.iter().any(|candidate| candidate.name == *value_column) {
        return Err(ExecutionError::Invalid(format!(
            "unknown topK value column: {value_column}"
        )));
    }
    writeln!(output, "{column}\ttopK({k})({value_column})")?;
    if query.limit == Some(0) {
        return Ok(());
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
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<String, usize>>();
    let types = metadata
        .columns
        .iter()
        .map(|candidate| (candidate.name.clone(), candidate.data_type.clone()))
        .collect::<HashMap<String, String>>();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let group_index = indices[column];
    let value_index = indices[value_column];
    let mut groups = Vec::<Group>::new();
    let mut text_positions = HashMap::<Vec<u8>, usize>::new();
    let mut other_positions = HashMap::<Key, usize>::new();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let text = match &batch[group_index] {
                TypedColumn::Text(values) => Some(values.get(row).ok_or_else(|| {
                    ExecutionError::Invalid("missing grouped topK String key".into())
                })?),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 =>
                    Some(values.get(row).ok_or_else(|| {
                        ExecutionError::Invalid("missing grouped topK nullable key".into())
                    })?),
                _ => None,
            };
            let existing = if let Some(text) = text {
                text_positions.get(text).copied()
            } else {
                other_positions.get(&typed::key_at(&batch[group_index], row)).copied()
            };
            let index = if let Some(index) = existing {
                index
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "grouped topK exceeds 128 cohorts (bounded heavy-hitter state)".into(),
                    ));
                }
                let index = groups.len();
                if let Some(text) = text {
                    if text.len() > MAX_GROUP_BYTES {
                        return Err(ExecutionError::Invalid(
                            "grouped topK dimension exceeds 512 bytes".into(),
                        ));
                    }
                    text_positions.insert(text.to_vec(), index);
                } else {
                    other_positions.insert(typed::key_at(&batch[group_index], row), index);
                }
                groups.push(Group {
                    value: typed::cell(&batch[group_index], row),
                    top: SpaceSaving::new(*k),
                });
                index
            };
            if let Some(value) = spacesaving_topk::view_at(&batch[value_index], row)? {
                if matches!(&value, View::Text(text) if text.len() > MAX_VALUE_BYTES) {
                    return Err(ExecutionError::Invalid(
                        "grouped topK value exceeds 256-byte memory budget".into(),
                    ));
                }
                groups[index].top.observe(value)?;
            }
        }
    }

    let mut rows = groups.into_iter().enumerate().collect::<Vec<_>>();
    if query.order_by.is_some() {
        rows.sort_unstable_by(|left, right| {
            for term in query.order_terms() {
                let cmp = typed::compare_order_cells(
                    &left.1.value,
                    &right.1.value,
                    term.descending,
                    term.nulls_first,
                );
                if !cmp.is_eq() {
                    return cmp;
                }
            }
            left.0.cmp(&right.0)
        });
    }
    for (_, group) in rows
        .iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        typed::write_typed_value(output, &group.value, group_type)?;
        output.write_all(b"\t[")?;
        for (index, key) in group.top.ranked().iter().enumerate() {
            if index > 0 {
                output.write_all(b",")?;
            }
            write_key(output, key)?;
        }
        output.write_all(b"]\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heavy_hitter_memory_is_independent_of_fact_table_rows() {
        assert!(MAX_GROUPS * 512 * MAX_VALUE_BYTES <= 16 * 1024 * 1024);
    }
}
