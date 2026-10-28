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
const WIDE_BITMAP_THRESHOLD: usize = 512;
const MAX_DENSE_GROUPS: usize = 8192; // 64 MiB of exact 16-bit bitmap payloads.

struct Group {
    key: Cell,
    distinct: HashSet<Key>,
    distinct_text: HashSet<Vec<u8>>,
    distinct_uuids: HashSet<[u8; 16]>,
    // Exact bounded 8-bit cardinality uses 32 bytes per group, regardless
    // of the number of rows or repeated distinct pairs.
    byte_values: [u64; 4],
    wide_distinct: HashSet<u16>,
    wide_bitmap: Option<Box<[u64; 1024]>>,
}

impl Group {
    fn distinct_len(&self) -> usize {
        self.distinct.len()
            + self.distinct_text.len()
            + self.distinct_uuids.len()
            + self.byte_values.iter().map(|word| word.count_ones() as usize).sum::<usize>()
            + self.wide_distinct.len()
            + self.wide_bitmap.as_ref().map_or(0, |bitmap| {
                bitmap.iter().map(|word| word.count_ones() as usize).sum::<usize>()
            })
    }
}

fn mark_distinct_byte(group: &mut Group, value: u8) {
    let slot = usize::from(value) / 64;
    group.byte_values[slot] |= 1u64 << (usize::from(value) % 64);
}

fn register_word_distinct(
    group: &mut Group,
    value: u16,
    total_pairs: &mut usize,
    dense_groups: &mut usize,
) -> Result<(), ExecutionError> {
    let slot = usize::from(value);
    if let Some(bitmap) = group.wide_bitmap.as_mut() {
        bitmap[slot / 64] |= 1u64 << (slot % 64);
        return Ok(());
    }
    if group.wide_distinct.contains(&value) {
        return Ok(());
    }
    if group.wide_distinct.len() >= WIDE_BITMAP_THRESHOLD && *dense_groups < MAX_DENSE_GROUPS {
        let mut bitmap = Box::new([0u64; 1024]);
        // A sparse key converted to a bitmap no longer consumes one of the
        // generic hash-pair slots. This permits exact dense high-cardinality
        // groups without raising the general memory budget.
        for old in group.wide_distinct.drain() {
            let index = usize::from(old);
            bitmap[index / 64] |= 1u64 << (index % 64);
            *total_pairs -= 1;
        }
        bitmap[slot / 64] |= 1u64 << (slot % 64);
        group.wide_bitmap = Some(bitmap);
        *dense_groups += 1;
    } else {
        if *total_pairs >= MAX_GROUP_VALUE_PAIRS {
            return Err(ExecutionError::Invalid(
                "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into(),
            ));
        }
        group.wide_distinct.insert(value);
        *total_pairs += 1;
    }
    Ok(())
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
    let byte_distinct = matches!(typed::base_type(types[value_column].as_str()), "UInt8" | "Int8");
    let word_distinct = matches!(
        typed::base_type(types[value_column].as_str()),
        "UInt16" | "Int16"
    );
    let mut dense_word_groups = 0usize;
    let mut total_pairs = 0usize;
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();

    while let Some(batch) = reader.next_batch()? {
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
                        byte_values: [0; 4],
                        wide_distinct: HashSet::new(),
                        wide_bitmap: None,
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
                        byte_values: [0; 4],
                        wide_distinct: HashSet::new(),
                        wide_bitmap: None,
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
                        byte_values: [0; 4],
                        wide_distinct: HashSet::new(),
                        wide_bitmap: None,
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
                        byte_values: [0; 4],
                        wide_distinct: HashSet::new(),
                        wide_bitmap: None,
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
                        byte_values: [0; 4],
                        wide_distinct: HashSet::new(),
                        wide_bitmap: None,
                        });
                        id
                    }
                }
            };

            // This is an exact UInt8/Int8 COUNT(DISTINCT) fast path.
            // Unlike a hash set, 256 possible values take constant memory.
            // Do not apply the generic 200k-pair cap to these fixed bitmaps.
            if byte_distinct {
                let value = match &batch[value_index] {
                    TypedColumn::Numeric(NumericColumn::UInt8(values)) => Some(values[row]),
                    TypedColumn::Numeric(NumericColumn::Int8(values)) => Some(values[row] as u8),
                    TypedColumn::NullableNumeric {
                        values: NumericColumn::UInt8(values),
                        nulls,
                    } if nulls[row] == 0 => Some(values[row]),
                    TypedColumn::NullableNumeric {
                        values: NumericColumn::Int8(values),
                        nulls,
                    } if nulls[row] == 0 => Some(values[row] as u8),
                    TypedColumn::NullableNumeric { nulls, .. } if nulls[row] != 0 => None,
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected byte-width COUNT(DISTINCT) type".into(),
                        ));
                    }
                };
                if let Some(value) = value {
                    mark_distinct_byte(&mut groups[id], value);
                }
                continue;
            }

            if word_distinct {
                let value = match &batch[value_index] {
                    TypedColumn::Numeric(NumericColumn::UInt16(values)) => Some(values[row]),
                    TypedColumn::Numeric(NumericColumn::Int16(values)) => Some(values[row] as u16),
                    TypedColumn::NullableNumeric { values: NumericColumn::UInt16(values), nulls }
                        if nulls[row] == 0 => Some(values[row]),
                    TypedColumn::NullableNumeric { values: NumericColumn::Int16(values), nulls }
                        if nulls[row] == 0 => Some(values[row] as u16),
                    TypedColumn::NullableNumeric { nulls, .. } if nulls[row] != 0 => None,
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected 16-bit COUNT(DISTINCT) type".into(),
                        ));
                    }
                };
                if let Some(value) = value {
                    register_word_distinct(
                        &mut groups[id],
                        value,
                        &mut total_pairs,
                        &mut dense_word_groups,
                    )?;
                }
                continue;
            }

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

#[cfg(test)]
mod grouped_byte_distinct_tests {
    use super::*;

    #[test]
    fn fixed_bitmap_counts_full_unsigned_and_signed_byte_domains() {
        let mut group = Group {
            key: Cell::Null,
            distinct: HashSet::new(),
            distinct_text: HashSet::new(),
            distinct_uuids: HashSet::new(),
            byte_values: [0; 4],
                        wide_distinct: HashSet::new(),
                        wide_bitmap: None,
        };
        for value in 0u8..=255 {
            mark_distinct_byte(&mut group, value);
            mark_distinct_byte(&mut group, value);
        }
        assert_eq!(group.distinct_len(), 256);
        assert_eq!(group.byte_values, [u64::MAX; 4]);
        assert_eq!(std::mem::size_of_val(&group.byte_values), 32);
    }

    #[test]
    fn bitmap_keeps_high_bit_and_zero_in_separate_slots() {
        let mut group = Group {
            key: Cell::Null,
            distinct: HashSet::new(),
            distinct_text: HashSet::new(),
            distinct_uuids: HashSet::new(),
            byte_values: [0; 4],
                        wide_distinct: HashSet::new(),
                        wide_bitmap: None,
        };
        mark_distinct_byte(&mut group, 0);
        mark_distinct_byte(&mut group, 128);
        mark_distinct_byte(&mut group, 255);
        mark_distinct_byte(&mut group, 128);
        assert_eq!(group.distinct_len(), 3);
    }
}

#[cfg(test)]
mod adaptive_grouped_word_distinct_tests {
    use super::*;

    fn new_group() -> Group {
        Group {
            key: Cell::Null,
            distinct: HashSet::new(),
            distinct_text: HashSet::new(),
            distinct_uuids: HashSet::new(),
            byte_values: [0; 4],
            wide_distinct: HashSet::new(),
            wide_bitmap: None,
        }
    }

    #[test]
    fn promotes_dense_u16_groups_and_keeps_exact_cardinality() {
        let mut group = new_group();
        let mut sparse_pairs = 0;
        let mut dense_groups = 0;
        for value in 0..=u16::MAX {
            register_word_distinct(&mut group, value, &mut sparse_pairs, &mut dense_groups).unwrap();
            register_word_distinct(&mut group, value, &mut sparse_pairs, &mut dense_groups).unwrap();
        }
        assert_eq!(group.distinct_len(), 65_536);
        assert_eq!(sparse_pairs, 0);
        assert_eq!(dense_groups, 1);
        assert!(group.wide_distinct.is_empty());
        assert!(group.wide_bitmap.is_some());
    }

    #[test]
    fn sparse_groups_count_towards_existing_pair_limit() {
        let mut group = new_group();
        let mut pairs = MAX_GROUP_VALUE_PAIRS;
        let mut dense = 0;
        assert!(register_word_distinct(&mut group, 3, &mut pairs, &mut dense).is_err());
        assert_eq!(pairs, MAX_GROUP_VALUE_PAIRS);
    }
}
