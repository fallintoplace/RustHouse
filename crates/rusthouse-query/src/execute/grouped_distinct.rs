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

const MAX_TEXT_BYTES: usize = 64 * 1024 * 1024;
const MAX_GROUP_STATE_BYTES: usize = 64 * 1024 * 1024;

fn group_limit_error() -> ExecutionError {
    ExecutionError::Invalid("GROUP BY supports at most 200000 groups".into())
}

fn pair_limit_error() -> ExecutionError {
    ExecutionError::Invalid("grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into())
}

fn charge_group_fixed(used: &mut usize) -> Result<(), ExecutionError> {
    // Account for the three HashSet headers, byte-value bitmap, Group row,
    // and the key lookup entry, even when no distinct payloads are present.
    let extra = std::mem::size_of::<Group>()
        .checked_add(std::mem::size_of::<(Key, usize)>())
        .ok_or_else(|| ExecutionError::Invalid("grouped DISTINCT state size overflow".into()))?;
    let next = used.checked_add(extra)
        .ok_or_else(|| ExecutionError::Invalid("grouped DISTINCT state size overflow".into()))?;
    if next > MAX_GROUP_STATE_BYTES {
        return Err(ExecutionError::Invalid(
            "grouped DISTINCT exceeds 64 MiB fixed group state budget".into(),
        ));
    }
    *used = next;
    Ok(())
}

fn charge_text(used: &mut usize, length: usize, copies: usize) -> Result<(), ExecutionError> {
    let added = length.checked_mul(copies)
        .ok_or_else(|| ExecutionError::Invalid("grouped DISTINCT String memory overflow".into()))?;
    let next = used.checked_add(added)
        .ok_or_else(|| ExecutionError::Invalid("grouped DISTINCT String memory overflow".into()))?;
    if next > MAX_TEXT_BYTES {
        return Err(ExecutionError::Invalid(
            "grouped DISTINCT exceeds 64 MiB String payload budget".into(),
        ));
    }
    *used = next;
    Ok(())
}


struct Group {
    key: Cell,
    distinct: HashSet<Key>,
    distinct_text: HashSet<Vec<u8>>,
    distinct_uuids: HashSet<[u8; 16]>,
    // Exact bounded 8-bit cardinality uses 32 bytes per group, regardless
    // of the number of rows or repeated distinct pairs.
    byte_values: [u64; 4],
}

impl Group {
    fn distinct_len(&self) -> usize {
        self.distinct.len()
            + self.distinct_text.len()
            + self.distinct_uuids.len()
            + self.byte_values.iter().map(|word| word.count_ones() as usize).sum::<usize>()
    }
}

fn mark_distinct_byte(group: &mut Group, value: u8) {
    let slot = usize::from(value) / 64;
    group.byte_values[slot] |= 1u64 << (usize::from(value) % 64);
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
    let mut total_pairs = 0usize;
    let mut retained_text_bytes = 0usize;
    let mut retained_group_bytes = 0usize;
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
                    charge_group_fixed(&mut retained_group_bytes)?;
                    groups.push(Group {
                        key: typed::cell(&batch[group_index], row),
                        distinct: HashSet::new(),
                        distinct_text: HashSet::new(),
                        distinct_uuids: HashSet::new(),
                        byte_values: [0; 4],
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
                    charge_group_fixed(&mut retained_group_bytes)?;
                    groups.push(Group {
                        key: typed::cell(&batch[group_index], row),
                        distinct: HashSet::new(),
                        distinct_text: HashSet::new(),
                        distinct_uuids: HashSet::new(),
                        byte_values: [0; 4],
                    });
                    id
                }
            } else if let Some(key) = group_uuid {
                if let Some(&id) = uuid_groups.get(&key) {
                    id
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(group_limit_error());
                    }
                    let id = groups.len();
                    uuid_groups.insert(key, id);
                    charge_group_fixed(&mut retained_group_bytes)?;
                    groups.push(Group {
                        key: typed::cell(&batch[group_index], row),
                        distinct: HashSet::new(),
                        distinct_text: HashSet::new(),
                        distinct_uuids: HashSet::new(),
                        byte_values: [0; 4],
                    });
                    id
                }
            } else if let Some(text) = group_text {
                if let Some(&id) = text_groups.get(text) {
                    id
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(group_limit_error());
                    }
                    charge_text(&mut retained_text_bytes, text.len(), 2)?;
                    let id = groups.len();
                    text_groups.insert(text.to_vec(), id);
                    charge_group_fixed(&mut retained_group_bytes)?;
                    groups.push(Group {
                        key: Cell::Text(text.to_vec()),
                        distinct: HashSet::new(),
                        distinct_text: HashSet::new(),
                        distinct_uuids: HashSet::new(),
                        byte_values: [0; 4],
                    });
                    id
                }
            } else {
                let group = typed::cell(&batch[group_index], row);
                match by_key.entry(group.key()) {
                    Entry::Occupied(entry) => *entry.get(),
                    Entry::Vacant(entry) => {
                        if groups.len() >= MAX_GROUPS {
                            return Err(group_limit_error());
                        }
                        let id = groups.len();
                        entry.insert(id);
                        charge_group_fixed(&mut retained_group_bytes)?;
                        groups.push(Group {
                            key: group,
                            distinct: HashSet::new(),
                            distinct_text: HashSet::new(),
                            distinct_uuids: HashSet::new(),
                        byte_values: [0; 4],
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

            // UUIDs are fixed-width values; avoid formatting every row to
            // a 36-byte String before hashing the distinct key.
            if let TypedColumn::Uuid(values) = &batch[value_index] {
                let key = *values[row].as_bytes();
                if total_pairs >= MAX_GROUP_VALUE_PAIRS {
                    if !groups[id].distinct_uuids.contains(&key) {
                        return Err(pair_limit_error());
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
                        return Err(pair_limit_error());
                    }
                    charge_text(&mut retained_text_bytes, text.len(), 1)?;
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
                    return Err(pair_limit_error());
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
        };
        mark_distinct_byte(&mut group, 0);
        mark_distinct_byte(&mut group, 128);
        mark_distinct_byte(&mut group, 255);
        mark_distinct_byte(&mut group, 128);
        assert_eq!(group.distinct_len(), 3);
    }
}

#[cfg(test)]
mod wide_grouped_string_budget_tests {
    use super::*;
    #[test]
    fn wide_group_and_distinct_value_budgets_fail_without_changing_state() {
        let mut used = 0usize;
        charge_text(&mut used, 8 * 1024 * 1024, 2).unwrap();
        assert_eq!(used, 16 * 1024 * 1024);
        assert!(charge_text(&mut used, 25 * 1024 * 1024, 2).is_err());
        assert_eq!(used, 16 * 1024 * 1024);
        charge_text(&mut used, 48 * 1024 * 1024, 1).unwrap();
        assert_eq!(used, MAX_TEXT_BYTES);
    }
}

#[cfg(test)]
mod fixed_group_memory_budget_tests {
    use super::*;

    #[test]
    fn newly_created_groups_charge_all_fixed_state() {
        let mut bytes = 0usize;
        charge_group_fixed(&mut bytes).unwrap();
        assert!(bytes >= std::mem::size_of::<Group>());
        let one = bytes;
        charge_group_fixed(&mut bytes).unwrap();
        assert_eq!(bytes, one * 2);
        bytes = MAX_GROUP_STATE_BYTES - 1;
        assert!(charge_group_fixed(&mut bytes).is_err());
        assert_eq!(bytes, MAX_GROUP_STATE_BYTES - 1);
    }
}
