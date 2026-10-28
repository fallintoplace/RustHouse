//! Grouped COUNT(DISTINCT column) over a bounded typed part scan.
use super::ExecutionError;
use super::typed::{self, Cell, Key};
use crate::Select;
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 100_000;
const MAX_GROUP_VALUE_PAIRS: usize = 100_000;

struct Group {
    key: Cell,
    distinct: HashSet<Key>,
    distinct_text: HashSet<Vec<u8>>,
    distinct_uuids: HashSet<[u8; 16]>,
}

impl Group {
    fn distinct_len(&self) -> usize {
        self.distinct.len() + self.distinct_text.len() + self.distinct_uuids.len()
    }
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
            .clone()
            .unwrap_or_else(|| format!("countDistinct({value_column})"))
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let group_index = indices[column];
    let value_index = indices[value_column];
    let mut groups = Vec::<Group>::new();
    let mut by_key = HashMap::<Key, usize>::new();
    // Repeated String group keys can be checked without owning row bytes.
    let mut text_groups = HashMap::<Vec<u8>, usize>::new();
    let mut total_pairs = 0usize;
    let mut reader = TypedQueryReader::open(part, &names)?;

    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            // Register the group even when every aggregate value is NULL.
            let group_text = match &batch[group_index] {
                TypedColumn::Text(values) => values.get(row),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => values.get(row),
                _ => None,
            };
            let id = if let Some(text) = group_text {
                if let Some(&id) = text_groups.get(text) {
                    id
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 100000 groups".into(),
                        ));
                    }
                    let id = groups.len();
                    text_groups.insert(text.to_vec(), id);
                    groups.push(Group {
                        key: Cell::Text(text.to_vec()),
                        distinct: HashSet::new(),
                        distinct_text: HashSet::new(),
                        distinct_uuids: HashSet::new(),
                    });
                    id
                }
            } else {
                let group = typed::cell(&batch[group_index], row);
                match by_key.entry(group.key()) {
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
                            distinct_text: HashSet::new(),
                        distinct_uuids: HashSet::new(),
                        });
                        id
                    }
                }
            };

            // UUIDs are fixed-width values; avoid formatting every row to
            // a 36-byte String before hashing the distinct key.
            if let TypedColumn::Uuid(values) = &batch[value_index] {
                let key = *values[row].as_bytes();
                if !groups[id].distinct_uuids.contains(&key) {
                    if total_pairs >= MAX_GROUP_VALUE_PAIRS {
                        return Err(ExecutionError::Invalid(
                            "grouped COUNT(DISTINCT) supports at most 100000 distinct pairs".into(),
                        ));
                    }
                    groups[id].distinct_uuids.insert(key);
                    total_pairs += 1;
                }
                continue;
            }
            let value_text = match &batch[value_index] {
                TypedColumn::Text(values) => values.get(row),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => values.get(row),
                _ => None,
            };
            if let Some(text) = value_text {
                // Probe decoded bytes. Own them only for new distinct pairs.
                if !groups[id].distinct_text.contains(text) {
                    if total_pairs >= MAX_GROUP_VALUE_PAIRS {
                        return Err(ExecutionError::Invalid(
                            "grouped COUNT(DISTINCT) supports at most 100000 distinct pairs".into(),
                        ));
                    }
                    groups[id].distinct_text.insert(text.to_vec());
                    total_pairs += 1;
                }
                continue;
            }
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
                    let comparison = left.distinct_len().cmp(&right.distinct_len());
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
        writeln!(output, "\t{}", group.distinct_len())?;
    }
    Ok(())
}
