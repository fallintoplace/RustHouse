use super::ExecutionError;
use super::typed::{self, Cell, Key};
use crate::Select;
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{HashMap, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 100_000;

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
    writeln!(
        output,
        "{}\t{}",
        columns.join("\t"),
        query.alias.as_deref().unwrap_or("count")
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let mut reader = TypedQueryReader::open(part, &names)?;
    let mut groups: Vec<(Vec<Cell>, u64)> = Vec::new();
    let mut by_key: HashMap<Vec<Key>, usize> = HashMap::new();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let values: Vec<_> = columns
                .iter()
                .map(|column| typed::cell(&batch[indices[column]], row))
                .collect();
            let key: Vec<_> = values.iter().map(Cell::key).collect();
            match by_key.entry(key) {
                Entry::Occupied(entry) => {
                    let count = &mut groups[*entry.get()].1;
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
                }
                Entry::Vacant(entry) => {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 100000 groups".into(),
                        ));
                    }
                    entry.insert(groups.len());
                    groups.push((values, 1));
                }
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
        groups.sort_by(|(left_keys, left_count), (right_keys, right_count)| {
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
        });
    }
    for (keys, count) in groups
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (index, value) in keys.iter().enumerate() {
            if index != 0 {
                output.write_all(b"\t")?;
            }
            typed::write_typed_value(output, value, &types[&columns[index]])?;
        }
        writeln!(output, "\t{count}")?;
    }
    Ok(())
}
