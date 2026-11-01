//! Exact COUNT(DISTINCT) segmented by multiple grouping dimensions.
use super::typed::{self, Cell, Key};
use super::{ExecutionError, Number};
use crate::Select;
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{HashMap, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;
const MAX_PAIRS: usize = 200_000;

const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;

/// Include the Group container, its fixed 256-bit byte-domain bitmap, and
/// the hash-map entry. The old estimate only charged projected key cells.
fn fixed_group_bytes(columns: usize) -> Result<usize, ExecutionError> {
    std::mem::size_of::<Group>()
        .checked_add(std::mem::size_of::<(Vec<Key>, usize)>())
        .and_then(|base| {
            columns
                .checked_mul(std::mem::size_of::<Key>() + std::mem::size_of::<Cell>())
                .and_then(|keys| base.checked_add(keys))
        })
        .ok_or_else(|| ExecutionError::Invalid("grouped DISTINCT memory overflow".into()))
}

fn charge_bytes(used: &mut usize, bytes: usize) -> Result<(), ExecutionError> {
    let next = used
        .checked_add(bytes)
        .ok_or_else(|| ExecutionError::Invalid("grouped DISTINCT memory overflow".into()))?;
    if next > MAX_RETAINED_BYTES {
        return Err(ExecutionError::Invalid(
            "grouped DISTINCT exceeds 64 MiB retained budget".into(),
        ));
    }
    *used = next;
    Ok(())
}

struct Group {
    keys: Vec<Cell>,
    values: HashMap<Key, ()>,
    // Exact cardinality of a byte-wide column needs only 256 bits per group.
    byte_values: [u64; 4],
    distinct_count: usize,
}

impl Group {
    fn cardinality(&self) -> usize {
        self.distinct_count
    }

    fn mark_byte(&mut self, value: u8) {
        let word = &mut self.byte_values[usize::from(value) / 64];
        let mask = 1u64 << (usize::from(value) % 64);
        if *word & mask == 0 {
            *word |= mask;
            self.distinct_count += 1;
        }
    }
}

fn numeric_byte_at(column: &TypedColumn, row: usize) -> Option<u8> {
    let values = match column {
        TypedColumn::Numeric(values) | TypedColumn::NullableNumeric { values, .. } => values,
        _ => return None,
    };
    match values {
        NumericColumn::UInt8(values) => Some(values[row]),
        NumericColumn::Int8(values) => Some(values[row] as u8),
        _ => None,
    }
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    columns: &[String],
    value_column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    let mut names = columns.to_vec();
    if !names.iter().any(|column| column == value_column) {
        names.push(value_column.to_owned());
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !names.iter().any(|column| column == predicate.column()) {
            names.push(predicate.column().to_owned());
        }
    }
    for name in &names {
        if !types.contains_key(name) {
            return Err(ExecutionError::Invalid(format!("unknown column: {name}")));
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    writeln!(
        output,
        "{}\tcountDistinct({value_column})",
        columns.join("\t")
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let group_indices = columns.iter().map(|name| indices[name]).collect::<Vec<_>>();
    let value_index = indices[value_column];
    let byte_values = matches!(
        typed::base_type(types[value_column].as_str()),
        "UInt8" | "Int8"
    );
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let mut groups = Vec::<Group>::new();
    let mut lookup = HashMap::<Vec<Key>, usize>::new();
    let mut probes = Vec::<Key>::with_capacity(columns.len());
    let mut total_pairs = 0usize;
    let mut retained_bytes = 0usize;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            probes.clear();
            for &index in &group_indices {
                probes.push(typed::key_at(&batch[index], row));
            }
            let group = if let Some(&index) = lookup.get(probes.as_slice()) {
                index
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 200000 groups".into(),
                    ));
                }
                // Hash keys and projected group cells each own variable-width data.
                let fixed = fixed_group_bytes(columns.len())?;
                let bytes = probes.iter().try_fold(fixed, |total, key| {
                    let dynamic = match key {
                        Key::Text(text) => text.len().checked_mul(2),
                        Key::Uuid(text) => text.len().checked_mul(2),
                        _ => Some(0),
                    }
                    .ok_or_else(|| {
                        ExecutionError::Invalid("grouped DISTINCT memory overflow".into())
                    })?;
                    total.checked_add(dynamic).ok_or_else(|| {
                        ExecutionError::Invalid("grouped DISTINCT memory overflow".into())
                    })
                })?;
                charge_bytes(&mut retained_bytes, bytes)?;
                let index = groups.len();
                lookup.insert(std::mem::take(&mut probes), index);
                probes = Vec::with_capacity(columns.len());
                groups.push(Group {
                    keys: group_indices
                        .iter()
                        .map(|&i| typed::cell(&batch[i], row))
                        .collect(),
                    values: HashMap::new(),
                    byte_values: [0; 4],
                    distinct_count: 0,
                });
                index
            };
            if typed::is_null_at(&batch[value_index], row) {
                continue;
            }
            if byte_values {
                let byte = numeric_byte_at(&batch[value_index], row).ok_or_else(|| {
                    ExecutionError::Invalid("expected byte-width DISTINCT value".into())
                })?;
                groups[group].mark_byte(byte);
                continue;
            }
            let value = typed::key_at(&batch[value_index], row);
            // An Entry probes the table once. contains() followed by insert()
            // hashed every new DISTINCT value twice.
            if let Entry::Vacant(slot) = groups[group].values.entry(value) {
                if total_pairs >= MAX_PAIRS {
                    return Err(ExecutionError::Invalid(
                        "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into(),
                    ));
                }
                let dynamic = match slot.key() {
                    Key::Text(text) => text.len(),
                    Key::Uuid(text) => text.len(),
                    _ => 0,
                };
                charge_bytes(
                    &mut retained_bytes,
                    std::mem::size_of::<Key>()
                        .checked_add(dynamic)
                        .ok_or_else(|| {
                            ExecutionError::Invalid("grouped DISTINCT memory overflow".into())
                        })?,
                )?;
                slot.insert(());
                groups[group].distinct_count += 1;
                total_pairs += 1;
            }
        }
    }
    if let Some(having) = &query.having {
        groups.retain(|group| {
            super::predicates::number_matches(
                Number::Unsigned(group.cardinality() as u128),
                Number::Unsigned(having.value as u128),
                having.operator,
            )
        });
    }
    let mut bounded_order = None;
    if query.order_by.is_some() {
        let terms = query
            .order_terms()
            .map(|term| {
                let index = columns.iter().position(|key| key == &term.column);
                (index, term.descending, term.nulls_first)
            })
            .collect::<Vec<_>>();
        let compare = |a: &Group, b: &Group| {
            for &(index, desc, nulls_first) in &terms {
                let comparison = if let Some(index) = index {
                    typed::compare_order_cells(&a.keys[index], &b.keys[index], desc, nulls_first)
                } else {
                    let order = a.cardinality().cmp(&b.cardinality());
                    if desc { order.reverse() } else { order }
                };
                if comparison != Ordering::Equal {
                    return comparison;
                }
            }
            Ordering::Equal
        };
        // Keep only OFFSET + LIMIT positions when the requested page is
        // smaller than the group set. Stable position ties preserve the
        // original full-sort ordering without an O(G log G) sort.
        if let Some(end) = query
            .limit
            .and_then(|limit| query.offset.unwrap_or(0).checked_add(limit))
            .filter(|&end| end < groups.len())
        {
            bounded_order = Some(super::sorted_group_positions(&groups, end, compare));
        } else {
            groups.sort_by(compare);
        }
    }
    let output_types = columns
        .iter()
        .map(|column| types[column].as_str())
        .collect::<Vec<_>>();
    let write_group = |group: &Group, output: &mut W| -> Result<(), ExecutionError> {
        for (index, cell) in group.keys.iter().enumerate() {
            if index > 0 {
                write!(output, "\t")?;
            }
            typed::write_typed_value(output, cell, output_types[index])?;
        }
        writeln!(output, "\t{}", group.cardinality())?;
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
mod bounded_payload_tests {
    use super::*;
    #[test]
    fn caches_exact_distinct_cardinality_for_sorting_and_having() {
        let mut group = Group {
            keys: Vec::new(),
            values: HashMap::new(),
            byte_values: [0; 4],
            distinct_count: 0,
        };
        for value in [0, 1, 255, 0, 64, 255, 128] {
            group.mark_byte(value);
        }
        assert_eq!(group.cardinality(), 5);
        assert_eq!(
            group
                .byte_values
                .iter()
                .map(|word| word.count_ones() as usize)
                .sum::<usize>(),
            group.cardinality()
        );
        group
            .values
            .insert(Key::Numeric(super::super::NumberKey::Unsigned(100)), ());
        group.distinct_count += 1;
        assert_eq!(group.cardinality(), 6);
    }

    #[test]
    fn fixed_group_budget_accounts_for_bitmap_and_hash_entry() {
        assert!(
            fixed_group_bytes(0).unwrap()
                >= std::mem::size_of::<Group>() + std::mem::size_of::<(Vec<Key>, usize)>()
        );
        assert_eq!(fixed_group_bytes(usize::MAX).is_err(), true);
        let mut charged = 0;
        charge_bytes(&mut charged, fixed_group_bytes(1).unwrap()).unwrap();
        assert!(charged >= std::mem::size_of::<Group>());
    }

    #[test]
    fn rejects_capacity_crossings_without_charging_failed_reservations() {
        let mut retained = 0usize;
        charge_bytes(&mut retained, MAX_RETAINED_BYTES - 17).unwrap();
        assert!(charge_bytes(&mut retained, 18).is_err());
        assert_eq!(retained, MAX_RETAINED_BYTES - 17);
        charge_bytes(&mut retained, 17).unwrap();
        assert_eq!(retained, MAX_RETAINED_BYTES);
    }
}

#[cfg(test)]
mod grouped_distinct_bounded_topk_tests {
    use super::*;
    use rusthouse_storage::{Column, write_wide_part};

    #[test]
    fn bounded_pages_match_stable_full_sort_for_multiple_order_terms() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-grouped-distinct-page-{}-{}",
            std::process::id(),
            line!(),
        ));
        let _ = std::fs::remove_dir_all(&root);
        let columns = [
            Column {
                name: "a".into(),
                data_type: "String".into(),
            },
            Column {
                name: "b".into(),
                data_type: "UInt8".into(),
            },
            Column {
                name: "v".into(),
                data_type: "UInt8".into(),
            },
        ];
        let rows = [
            ("a", 1, 1),
            ("a", 1, 2),
            ("a", 2, 1),
            ("b", 1, 1),
            ("b", 1, 2),
            ("b", 1, 3),
            ("b", 2, 1),
            ("c", 2, 2),
            ("c", 2, 3),
        ]
        .into_iter()
        .map(|(a, b, v)| vec![Some(a.to_owned()), Some(b.to_string()), Some(v.to_string())])
        .collect::<Vec<_>>();
        write_wide_part(&root, &columns, &rows).unwrap();

        for order in [
            "count DESC",
            "count DESC, a DESC, b DESC",
            "a ASC, count DESC",
        ] {
            let base =
                format!("SELECT a, b, COUNT(DISTINCT v) FROM part GROUP BY a, b ORDER BY {order}");
            let mut full = Vec::new();
            execute(
                &crate::parse_select(&base).unwrap(),
                &root,
                &["a".into(), "b".into()],
                "v",
                &mut full,
            )
            .unwrap();
            let full = String::from_utf8(full).unwrap();
            let all = full.lines().skip(1).collect::<Vec<_>>();
            for (offset, limit) in [(0, 1), (1, 2), (2, 1), (3, 2)] {
                let query =
                    crate::parse_select(&format!("{base} LIMIT {limit} OFFSET {offset}")).unwrap();
                let mut page = Vec::new();
                execute(&query, &root, &["a".into(), "b".into()], "v", &mut page).unwrap();
                let page = String::from_utf8(page).unwrap();
                assert_eq!(
                    page.lines().skip(1).collect::<Vec<_>>(),
                    all.iter()
                        .skip(offset)
                        .take(limit)
                        .copied()
                        .collect::<Vec<_>>(),
                    "{order}: OFFSET {offset} LIMIT {limit}"
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod single_probe_distinct_tests {
    use super::*;
    use rusthouse_storage::{Column, write_wide_part};

    #[test]
    fn counts_non_byte_distinct_values_once_per_group() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-multidistinct-entry-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let schema = [
            Column {
                name: "category".into(),
                data_type: "String".into(),
            },
            Column {
                name: "amount".into(),
                data_type: "UInt64".into(),
            },
        ];
        let rows = [
            ("a", 1),
            ("a", 1),
            ("a", 2),
            ("a", 2),
            ("b", 1),
            ("b", 1),
            ("b", 3),
        ]
        .into_iter()
        .map(|(category, amount)| vec![Some(category.into()), Some(amount.to_string())])
        .collect::<Vec<_>>();
        write_wide_part(&root, &schema, &rows).unwrap();
        let sql =
            "SELECT category, COUNT(DISTINCT amount) FROM part GROUP BY category ORDER BY category";
        let query = crate::parse_select(sql).unwrap();
        let mut output = Vec::new();
        execute(&query, &root, &["category".into()], "amount", &mut output).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "category\tcountDistinct(amount)\na\t2\nb\t2\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
