//! Bounded per-cohort latency percentiles over large analytical fact tables.
//! Each group owns one 8192-element reservoir and stores no original rows.
use super::quantile_sample::{CAPACITY, Reservoir};
use super::typed::{self, Cell, Key};
use super::{ExecutionError, Number, compare_numbers, number_at};
use crate::{Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 256;
const MAX_GROUP_NAME_BYTES: usize = 2048;

struct Group {
    group: Cell,
    sample: Reservoir,
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::GroupedQuantile { column, value_column, probability_ppm } = &query.projection
    else {
        unreachable!("expected grouped quantile");
    };
    let metadata = inspect_query_source(part)?;
    let group_type = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown group key: {column}")))?
        .data_type
        .as_str();
    let value_type = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *value_column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown quantile column: {value_column}")))?
        .data_type
        .as_str();
    let numeric_type = typed::base_type(value_type);
    if !(numeric_type.starts_with("UInt")
        || numeric_type.starts_with("Int")
        || numeric_type.starts_with("Float"))
    {
        return Err(ExecutionError::Invalid(
            "grouped quantile requires numeric value columns".into(),
        ));
    }
    let probability = f64::from(*probability_ppm) / 1_000_000.0;
    writeln!(output, "{column}\tquantile({probability})({value_column})")?;
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
    let kinds = metadata
        .columns
        .iter()
        .map(|candidate| (candidate.name.clone(), candidate.data_type.clone()))
        .collect::<HashMap<String, String>>();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &kinds)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &kinds)?);
    }
    let group_index = indices[column];
    let value_index = indices[value_column];
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let mut groups = Vec::<Group>::new();
    let mut strings = HashMap::<Vec<u8>, usize>::new();
    let mut others = HashMap::<Key, usize>::new();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let text = match &batch[group_index] {
                TypedColumn::Text(values) => Some(values.get(row).ok_or_else(|| {
                    ExecutionError::Invalid("invalid group String value".into())
                })?),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 =>
                    Some(values.get(row).ok_or_else(|| {
                        ExecutionError::Invalid("invalid nullable group String".into())
                    })?),
                _ => None,
            };
            let existing = if let Some(text) = text {
                strings.get(text).copied()
            } else {
                others.get(&typed::key_at(&batch[group_index], row)).copied()
            };
            let index = if let Some(existing) = existing {
                existing
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "grouped quantile exceeds 256 cohorts (bounded reservoir budget)".into(),
                    ));
                }
                let index = groups.len();
                if let Some(text) = text {
                    if text.len() > MAX_GROUP_NAME_BYTES {
                        return Err(ExecutionError::Invalid(
                            "grouped quantile String key exceeds 2048 bytes".into(),
                        ));
                    }
                    strings.insert(text.to_vec(), index);
                } else {
                    others.insert(typed::key_at(&batch[group_index], row), index);
                }
                groups.push(Group {
                    group: typed::cell(&batch[group_index], row),
                    sample: Reservoir::default(),
                });
                index
            };
            let value = match &batch[value_index] {
                TypedColumn::Numeric(values) => Some(number_at(values, row)),
                TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 =>
                    Some(number_at(values, row)),
                TypedColumn::NullableNumeric { .. } => None,
                _ => return Err(ExecutionError::Invalid(
                    "quantile requires typed numeric batches".into(),
                )),
            };
            if let Some(value) = value {
                groups[index].sample.observe(value)?;
            }
        }
    }

    let mut values = groups
        .into_iter()
        .enumerate()
        .map(|(order, mut group)| {
            (order, group.group, group.sample.quantile(*probability_ppm))
        })
        .collect::<Vec<_>>();
    if let Some(having) = &query.having {
        values.retain(|(_, _, estimate)| {
            estimate.is_some_and(|value| {
                super::predicates::number_matches(
                    value,
                    Number::Unsigned(u128::from(having.value)),
                    having.operator,
                )
            })
        });
    }
    if query.order_by.is_some() {
        values.sort_unstable_by(|left, right| {
            for term in query.order_terms() {
                let ordering = if term.column == "#quantile" {
                    match (left.2, right.2) {
                        (Some(a), Some(b)) => {
                            let order = compare_numbers(a, b);
                            if term.descending { order.reverse() } else { order }
                        }
                        (None, None) => Ordering::Equal,
                        (None, Some(_)) => if term.nulls_first.unwrap_or(false) {
                            Ordering::Less
                        } else {
                            Ordering::Greater
                        },
                        (Some(_), None) => if term.nulls_first.unwrap_or(false) {
                            Ordering::Greater
                        } else {
                            Ordering::Less
                        },
                    }
                } else {
                    typed::compare_order_cells(
                        &left.1,
                        &right.1,
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
    for (_, group, percentile) in values
        .iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        typed::write_typed_value(output, group, group_type)?;
        match percentile {
            Some(number) => writeln!(output, "\t{number}")?,
            None => writeln!(output, "\tNULL")?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_reservoir_budget_does_not_grow_with_rows() {
        assert!(MAX_GROUPS * CAPACITY * size_of::<Number>() <= 128 * 1024 * 1024);
    }
}
