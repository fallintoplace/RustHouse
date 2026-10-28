//! Bounded per-group HyperLogLog for multi-dimensional OLAP cohorts.
//! Probe borrowed typed group keys. Allocate owned keys only on first sight
//! of a dimension tuple; memory never scales with fact-table row count.
use super::hyperloglog::HyperLogLog;
use super::typed::{self, Cell, Key};
use super::{ExecutionError, Number, NumberKey, number_at};
use crate::{Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 4096;
const MAX_KEY_BYTES: usize = 256;

enum Borrowed<'a> {
    Number(NumberKey),
    Text(&'a [u8]),
    Uuid(&'a [u8; 16]),
    Null,
}

fn borrowed(column: &TypedColumn, row: usize) -> Result<Borrowed<'_>, ExecutionError> {
    match column {
        TypedColumn::Numeric(values) =>
            Ok(Borrowed::Number(NumberKey::from(number_at(values, row)))),
        TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 =>
            Ok(Borrowed::Number(NumberKey::from(number_at(values, row)))),
        TypedColumn::Text(values) =>
            values.get(row).map(Borrowed::Text).ok_or_else(|| {
                ExecutionError::Invalid("invalid GROUP BY String row".into())
            }),
        TypedColumn::NullableText { values, nulls } if nulls[row] == 0 =>
            values.get(row).map(Borrowed::Text).ok_or_else(|| {
                ExecutionError::Invalid("invalid nullable GROUP BY String row".into())
            }),
        TypedColumn::Uuid(values) => Ok(Borrowed::Uuid(values[row].as_bytes())),
        TypedColumn::NullableText { .. } | TypedColumn::NullableNumeric { .. } =>
            Ok(Borrowed::Null),
    }
}

fn hash_group(batch: &[TypedColumn], columns: &[usize], row: usize) -> Result<u64, ExecutionError> {
    let mut hash = DefaultHasher::new();
    for &index in columns {
        match borrowed(&batch[index], row)? {
            Borrowed::Number(value) => {
                1u8.hash(&mut hash);
                value.hash(&mut hash);
            }
            Borrowed::Text(value) => {
                2u8.hash(&mut hash);
                value.hash(&mut hash);
            }
            Borrowed::Uuid(value) => {
                3u8.hash(&mut hash);
                value.hash(&mut hash);
            }
            Borrowed::Null => 4u8.hash(&mut hash),
        }
    }
    Ok(hash.finish())
}

fn equal_group(
    stored: &[Key],
    batch: &[TypedColumn],
    columns: &[usize],
    row: usize,
) -> Result<bool, ExecutionError> {
    for (expected, &index) in stored.iter().zip(columns) {
        let same = match (expected, borrowed(&batch[index], row)?) {
            (Key::Numeric(expected), Borrowed::Number(actual)) => *expected == actual,
            (Key::Text(expected), Borrowed::Text(actual)) => expected == actual,
            (Key::UuidBytes(expected), Borrowed::Uuid(actual)) => expected == actual,
            (Key::Null, Borrowed::Null) => true,
            _ => false,
        };
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}

struct Group {
    keys: Vec<Key>,
    values: Vec<Cell>,
    sketch: HyperLogLog,
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::GroupedMultiApproxDistinct { columns, value_column } = &query.projection else {
        unreachable!("expected multi-dimensional grouped uniq");
    };
    let metadata = inspect_query_source(part)?;
    let mut names = columns.clone();
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
    for name in &names {
        if !metadata.columns.iter().any(|candidate| candidate.name == *name) {
            return Err(ExecutionError::Invalid(format!("unknown grouped uniq column: {name}")));
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<String, usize>>();
    let kinds = columns.iter().map(|name| {
        metadata.columns.iter().find(|candidate| candidate.name == *name)
            .expect("validated GROUP BY key").data_type.clone()
    }).collect::<Vec<_>>();
    let types = metadata
        .columns
        .iter()
        .map(|candidate| (candidate.name.clone(), candidate.data_type.clone()))
        .collect::<HashMap<String, String>>();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    write!(output, "{}\tuniq({value_column})\n", columns.join("\t"))?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let group_indices = columns.iter().map(|name| indices[name]).collect::<Vec<_>>();
    let value_index = indices[value_column];
    let mut seen = HashMap::<u64, Vec<usize>>::new();
    let mut groups = Vec::<Group>::new();
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let hash = hash_group(&batch, &group_indices, row)?;
            let mut found = None;
            if let Some(bucket) = seen.get(&hash) {
                for &index in bucket {
                    if equal_group(&groups[index].keys, &batch, &group_indices, row)? {
                        found = Some(index);
                        break;
                    }
                }
            }
            let index = if let Some(existing) = found {
                existing
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "multi-dimensional uniq exceeds 4096 groups (64 MiB sketch budget)".into(),
                    ));
                }
                let keys = group_indices
                    .iter()
                    .map(|&index| typed::key_at(&batch[index], row))
                    .collect::<Vec<_>>();
                if keys.iter().any(|key| {
                    matches!(key, Key::Text(value) if value.len() > MAX_KEY_BYTES)
                }) {
                    return Err(ExecutionError::Invalid(
                        "multi-dimensional uniq group key exceeds 256 bytes".into(),
                    ));
                }
                let values = group_indices
                    .iter()
                    .map(|&index| typed::cell(&batch[index], row))
                    .collect();
                let index = groups.len();
                groups.push(Group {
                    keys,
                    values,
                    sketch: HyperLogLog::default(),
                });
                seen.entry(hash).or_default().push(index);
                index
            };
            groups[index].sketch.observe(&batch[value_index], row)?;
        }
    }

    // Evaluate a sketch once per group, never within sort comparisons.
    let mut results = groups
        .into_iter()
        .enumerate()
        .map(|(order, group)| (order, group.values, group.sketch.estimate()))
        .collect::<Vec<_>>();
    if let Some(having) = &query.having {
        results.retain(|(_, _, estimate)| {
            super::predicates::number_matches(
                Number::Unsigned(u128::from(*estimate)),
                Number::Unsigned(u128::from(having.value)),
                having.operator,
            )
        });
    }
    if query.order_by.is_some() {
        results.sort_unstable_by(|left, right| {
            for term in query.order_terms() {
                let ordering = if term.column == "uniq" {
                    if term.descending {
                        right.2.cmp(&left.2)
                    } else {
                        left.2.cmp(&right.2)
                    }
                } else {
                    let position = columns.iter().position(|name| name == &term.column)
                        .expect("validated grouped key");
                    typed::compare_order_cells(
                        &left.1[position],
                        &right.1[position],
                        term.descending,
                        term.nulls_first,
                    )
                };
                if !ordering.is_eq() {
                    return ordering;
                }
            }
            left.0.cmp(&right.0)
        });
    }
    for (_, values, estimate) in results
        .iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (index, cell) in values.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\t")?;
            }
            typed::write_typed_value(output, cell, &kinds[index])?;
        }
        writeln!(output, "\t{estimate}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_groups_do_not_scale_with_fact_row_count() {
        assert_eq!(MAX_GROUPS * 16 * 1024, 64 * 1024 * 1024);
    }
}
