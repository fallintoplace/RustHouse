//! Grouped COUNT(DISTINCT column) over a bounded typed part scan.
use super::ExecutionError;
use super::typed::{self, Cell, Key};
use crate::Select;
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 100_000;
const MAX_GROUP_VALUE_PAIRS: usize = 100_000;

struct Group {
    key: Cell,
    distinct: HashSet<Key>,
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    value_column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut names = vec![column.to_owned()];
    if value_column != column {
        names.push(value_column.to_owned());
    }
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
        "{column}\t{}",
        query
            .alias
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("countDistinct({value_column})"))
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let group_index = indices[column];
    let value_index = indices[value_column];
    let mut groups = Vec::<Group>::new();
    let mut by_key = HashMap::<Key, usize>::new();
    let mut total_pairs = 0usize;
    let mut reader = TypedQueryReader::open(part, &names)?;

    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            // Register the group even when every aggregate value is NULL.
            let group = typed::cell(&batch[group_index], row);
            let id = match by_key.entry(group.key()) {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(entry) => {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 100000 groups".into(),
                        ));
                    }
                    let id = groups.len();
                    entry.insert(id);
                    groups.push(Group {
                        key: group,
                        distinct: HashSet::new(),
                    });
                    id
                }
            };

            let value = typed::cell(&batch[value_index], row);
            if matches!(&value, Cell::Null) {
                continue;
            }
            let key = value.key();
            if !groups[id].distinct.contains(&key) {
                if total_pairs >= MAX_GROUP_VALUE_PAIRS {
                    return Err(ExecutionError::Invalid(
                        "grouped COUNT(DISTINCT) supports at most 100000 distinct pairs".into(),
                    ));
                }
                groups[id].distinct.insert(key);
                total_pairs += 1;
            }
        }
    }

    if query.order_by.is_some() {
        groups.sort_by(|left, right| {
            for order in query.order_terms() {
                let comparison = if order.column == "count" {
                    let comparison = left.distinct.len().cmp(&right.distinct.len());
                    if order.descending {
                        comparison.reverse()
                    } else {
                        comparison
                    }
                } else {
                    typed::compare_order_cells(
                        &left.key,
                        &right.key,
                        order.descending,
                        order.nulls_first,
                    )
                };
                if comparison != Ordering::Equal {
                    return comparison;
                }
            }
            typed::cmp(&left.key, &right.key)
        });
    }

    for group in groups
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        typed::write_typed_value(output, &group.key, &types[column])?;
        writeln!(output, "\t{}", group.distinct.len())?;
    }
    Ok(())
}
