//! Fixed-memory per-group approximate cardinality for large fact tables.
//! At most 8192 active HLL sketches (128 MiB of register storage) may exist.
//! This is approximate; COUNT(DISTINCT) remains the exact alternative.
use super::hyperloglog::HyperLogLog;
use super::typed::{self, Cell, Key};
use super::{ExecutionError, Number};
use crate::{Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 8_192;

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::GroupedApproxDistinct {
        column,
        value_column,
    } = &query.projection else {
        unreachable!("grouped uniq requires a grouped approximation");
    };
    let metadata = inspect_query_source(part)?;
    let group_type = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown GROUP BY column: {column}")))?
        .data_type
        .as_str();
    if !metadata.columns.iter().any(|candidate| candidate.name == *value_column) {
        return Err(ExecutionError::Invalid(format!(
            "unknown uniq() column: {value_column}"
        )));
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
        .map(|candidate| (candidate.name.clone(), candidate.data_type.clone()))
        .collect();
    let mut filters = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        filters.push(typed::compile(branch, &indices, &types)?);
    }
    writeln!(output, "{column}\tuniq({value_column})")?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let group_index = indices[column];
    let value_index = indices[value_column];
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let mut text_positions = HashMap::<Vec<u8>, usize>::new();
    let mut other_positions = HashMap::<Key, usize>::new();
    let mut groups = Vec::<(Cell, HyperLogLog)>::new();

    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &filters) {
                continue;
            }
            // String groups are normally low cardinality but appear in every
            // fact row. Probe borrowed bytes and allocate only on new groups.
            let text = match &batch[group_index] {
                TypedColumn::Text(values) => Some(values.get(row).ok_or_else(|| {
                    ExecutionError::Invalid("missing GROUP BY String value".into())
                })?),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
                    Some(values.get(row).ok_or_else(|| {
                        ExecutionError::Invalid("missing nullable GROUP BY value".into())
                    })?)
                }
                _ => None,
            };
            let group = if let Some(text) = text {
                if let Some(&existing) = text_positions.get(text) {
                    existing
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "grouped uniq() exceeds 8192 groups (128 MiB sketch budget)".into(),
                        ));
                    }
                    let position = groups.len();
                    text_positions.insert(text.to_vec(), position);
                    groups.push((Cell::Text(text.to_vec()), HyperLogLog::default()));
                    position
                }
            } else {
                let key = typed::key_at(&batch[group_index], row);
                if let Some(&existing) = other_positions.get(&key) {
                    existing
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "grouped uniq() exceeds 8192 groups (128 MiB sketch budget)".into(),
                        ));
                    }
                    let position = groups.len();
                    other_positions.insert(key, position);
                    groups.push((
                        typed::cell(&batch[group_index], row),
                        HyperLogLog::default(),
                    ));
                    position
                }
            };
            groups[group].1.observe(&batch[value_index], row)?;
        }
    }

    // Estimate once per group. HLL estimates scan 16 KiB; computing them
    // inside sort comparisons would make large group ranking unnecessarily slow.
    let mut results = groups
        .into_iter()
        .map(|(key, sketch)| (key, sketch.estimate()))
        .collect::<Vec<_>>();
    if let Some(having) = &query.having {
        results.retain(|(_, count)| {
            super::predicates::number_matches(
                Number::Unsigned(u128::from(*count)),
                Number::Unsigned(u128::from(having.value)),
                having.operator,
            )
        });
    }
    if query.order_by.is_some() {
        results.sort_unstable_by(|left, right| {
            for term in query.order_terms() {
                let order = if term.column == "uniq" {
                    let value = left.1.cmp(&right.1);
                    if term.descending {
                        value.reverse()
                    } else {
                        value
                    }
                } else {
                    typed::compare_order_cells(
                        &left.0,
                        &right.0,
                        term.descending,
                        term.nulls_first,
                    )
                };
                if !order.is_eq() {
                    return order;
                }
            }
            typed::cmp(&left.0, &right.0)
        });
    }
    for (key, count) in results
        .iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        typed::write_typed_value(output, key, group_type)?;
        writeln!(output, "\t{count}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_sketch_memory_ceiling_is_explicit() {
        assert_eq!(MAX_GROUPS, 8192);
        assert_eq!(MAX_GROUPS * 16 * 1024, 128 * 1024 * 1024);
    }
}
