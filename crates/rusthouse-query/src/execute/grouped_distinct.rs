//! Grouped COUNT(DISTINCT column) over a bounded typed part scan.
use super::ExecutionError;
use super::typed::{self, Cell, Key};
use crate::Select;
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;
const MAX_GROUP_VALUE_PAIRS: usize = 200_000;

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


/// Scan disjoint immutable parts with exact group/value sets, then merge in
/// part order. Each partial result is bounded by the same exact-pair budget
/// as the serial executor and outputs the same first-seen group order.
fn parallel_grouped_distinct(
    part: &Path,
    names: &[String],
    group_index: usize,
    value_index: usize,
    conditions: &[Vec<typed::Condition>],
    no_filters: bool,
) -> Result<Option<Vec<Group>>, ExecutionError> {
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    let size = parts.len().div_ceil(workers);
    let chunks = std::thread::scope(|scope| {
        let handles = parts.chunks(size).map(|paths| {
            scope.spawn(move || -> Result<Vec<Group>, ExecutionError> {
                let mut groups: Vec<Group> = Vec::new();
                let mut by_key = HashMap::<Key, usize>::new();
                let mut pairs = 0usize;
                for path in paths {
                    let mut reader = rusthouse_storage::TypedPartReader::open(path, names)?;
                    while let Some(batch) = reader.next_batch()? {
                        for row in 0..batch[0].len() {
                            if !no_filters && !typed::passes(&batch, row, conditions) {
                                continue;
                            }
                            let key = typed::key_at(&batch[group_index], row);
                            let id = match by_key.entry(key) {
                                Entry::Occupied(entry) => *entry.get(),
                                Entry::Vacant(entry) => {
                                    if groups.len() >= MAX_GROUPS {
                                        return Err(ExecutionError::Invalid(
                                            "GROUP BY supports at most 200000 groups".into(),
                                        ));
                                    }
                                    let id = groups.len();
                                    entry.insert(id);
                                    groups.push(Group {
                                        key: typed::cell(&batch[group_index], row),
                                        distinct: HashSet::new(),
                                        distinct_text: HashSet::new(),
                                        distinct_uuids: HashSet::new(),
                                    });
                                    id
                                }
                            };
                            if typed::is_null_at(&batch[value_index], row) {
                                continue;
                            }
                            let inserted = match &batch[value_index] {
                                TypedColumn::Uuid(values) => {
                                    groups[id].distinct_uuids.insert(*values[row].as_bytes())
                                }
                                TypedColumn::Text(values) | TypedColumn::NullableText { values, .. } => {
                                    groups[id].distinct_text.insert(values.get(row).unwrap().to_vec())
                                }
                                column => {
                                    groups[id].distinct.insert(typed::key_at(column, row))
                                }
                            };
                            if inserted {
                                pairs += 1;
                                if pairs > MAX_GROUP_VALUE_PAIRS {
                                    return Err(ExecutionError::Invalid(
                                        "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into(),
                                    ));
                                }
                            }
                        }
                    }
                }
                Ok(groups)
            })
        }).collect::<Vec<_>>();
        handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| {
            Err(ExecutionError::Invalid("parallel distinct worker panicked".into()))
        })).collect::<Vec<_>>()
    });
    let mut groups: Vec<Group> = Vec::new();
    let mut by_key = HashMap::<Key, usize>::new();
    let mut pairs = 0usize;
    for chunk in chunks {
        for group in chunk? {
            let key = group.key.key();
            if let Some(&id) = by_key.get(&key) {
                let existing = &mut groups[id];
                let before = existing.distinct_len();
                existing.distinct.extend(group.distinct);
                existing.distinct_text.extend(group.distinct_text);
                existing.distinct_uuids.extend(group.distinct_uuids);
                pairs += existing.distinct_len() - before;
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 200000 groups".into(),
                    ));
                }
                pairs += group.distinct_len();
                by_key.insert(key, groups.len());
                groups.push(group);
            }
            if pairs > MAX_GROUP_VALUE_PAIRS {
                return Err(ExecutionError::Invalid(
                    "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into(),
                ));
            }
        }
    }
    Ok(Some(groups))
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
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let partial = parallel_grouped_distinct(
        part, &names, group_index, value_index, &conditions, no_filters,
    )?;
    let scan_serially = partial.is_none();
    let mut groups = partial.unwrap_or_default();
    let mut by_key = HashMap::<Key, usize>::new();
    // Repeated String group keys can be checked without owning row bytes.
    let mut text_groups = HashMap::<Vec<u8>, usize>::new();
    let mut uuid_groups = HashMap::<[u8; 16], usize>::new();
    // One byte yields at most 256 groups. Use direct lookup rather than
    // hashing every GROUP BY key in large low-cardinality scans.
    let mut dense_slots = matches!(
        types.get(column).map(String::as_str),
        Some("UInt8" | "Int8")
    )
    .then(|| [usize::MAX; 256]);
    // Byte-sized grouped DISTINCT is optimized separately. This path
    // specializes the larger 16-bit key domain.
    let mut dense_groups = matches!(types[column].as_str(), "UInt16" | "Int16")
        .then(|| vec![usize::MAX; 65_536]);
    let mut total_pairs = 0usize;
    let mut reader = if scan_serially {
        Some(TypedQueryReader::open_with_metadata(part, &names, &metadata)?)
    } else {
        None
    };

    while let Some(batch) = reader.as_mut().map_or(Ok(None), TypedQueryReader::next_batch)? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            // Register the group even when every aggregate value is NULL.
            let group_text = match &batch[group_index] {
                TypedColumn::Text(values) => values.get(row),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => values.get(row),
                _ => None,
            };
            let group_uuid = match &batch[group_index] {
                TypedColumn::Uuid(values) => Some(*values[row].as_bytes()),
                _ => None,
            };
            let id = if let Some(slots) = dense_slots.as_mut() {
                let byte = match &batch[group_index] {
                    TypedColumn::Numeric(NumericColumn::UInt8(values)) => values[row],
                    TypedColumn::Numeric(NumericColumn::Int8(values)) => values[row] as u8,
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected byte-width grouped DISTINCT column".into(),
                        ));
                    }
                };
                let slot = &mut slots[usize::from(byte)];
                if *slot == usize::MAX {
                    *slot = groups.len();
                    groups.push(Group {
                        key: typed::cell(&batch[group_index], row),
                        distinct: HashSet::new(),
                        distinct_text: HashSet::new(),
                        distinct_uuids: HashSet::new(),
                    });
                }
                *slot
            } else if let Some(slots) = dense_groups.as_mut() {
                let key = match &batch[group_index] {
                    TypedColumn::Numeric(rusthouse_storage::NumericColumn::UInt16(values)) => {
                        usize::from(values[row])
                    }
                    TypedColumn::Numeric(rusthouse_storage::NumericColumn::Int16(values)) => {
                        usize::from(values[row] as u16)
                    }
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected dense GROUP BY key type".into(),
                        ));
                    }
                };
                let slot = &mut slots[key];
                if *slot != usize::MAX {
                    *slot
                } else {
                    let id = groups.len();
                    *slot = id;
                    groups.push(Group {
                        key: typed::cell(&batch[group_index], row),
                        distinct: HashSet::new(),
                        distinct_text: HashSet::new(),
                        distinct_uuids: HashSet::new(),
                    });
                    id
                }
            } else if let Some(key) = group_uuid {
                if let Some(&id) = uuid_groups.get(&key) {
                    id
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 200000 groups".into(),
                        ));
                    }
                    let id = groups.len();
                    uuid_groups.insert(key, id);
                    groups.push(Group {
                        key: typed::cell(&batch[group_index], row),
                        distinct: HashSet::new(),
                        distinct_text: HashSet::new(),
                        distinct_uuids: HashSet::new(),
                    });
                    id
                }
            } else if let Some(text) = group_text {
                if let Some(&id) = text_groups.get(text) {
                    id
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 200000 groups".into(),
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
                                "GROUP BY supports at most 200000 groups".into(),
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
                if total_pairs >= MAX_GROUP_VALUE_PAIRS {
                    if !groups[id].distinct_uuids.contains(&key) {
                        return Err(ExecutionError::Invalid(
                            "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into(),
                        ));
                    }
                } else if groups[id].distinct_uuids.insert(key) {
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
                            "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into(),
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
            if total_pairs >= MAX_GROUP_VALUE_PAIRS {
                if !groups[id].distinct.contains(&key) {
                    return Err(ExecutionError::Invalid(
                        "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into(),
                    ));
                }
            } else if groups[id].distinct.insert(key) {
                total_pairs += 1;
            }
        }
    }

    if let Some(having) = &query.having {
        groups.retain(|group| {
            super::predicates::number_matches(
                super::Number::Unsigned(group.distinct_len() as u128),
                super::Number::Unsigned(u128::from(having.value)),
                having.operator,
            )
        });
    }

    let mut bounded_order = None;
    if query.order_by.is_some() {
        let compare = |left: &Group, right: &Group| {
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
        };
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

    let write_group = |group: &Group, output: &mut W| -> Result<(), ExecutionError> {
        typed::write_typed_value(output, &group.key, &types[column])?;
        writeln!(output, "\t{}", group.distinct_len())?;
        Ok(())
    };
    if let Some(indices) = bounded_order {
        for index in indices.into_iter().skip(query.offset.unwrap_or(0)) {
            write_group(&groups[index], output)?;
        }
    } else {
        for group in groups
            .iter()
            .skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX))
        {
            write_group(group, output)?;
        }
    }
    Ok(())
}
