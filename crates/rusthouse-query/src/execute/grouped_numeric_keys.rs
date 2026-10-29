//! Numeric aggregates grouped by multiple typed columns.
use super::typed::{self, Cell, Key};
use super::{AggregateState, ExecutionError, Number};
use crate::{AggregateFunction, Select};
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;

const MAX_RETAINED_GROUP_BYTES: usize = 64 * 1024 * 1024;

fn charge_median_value(used: &mut usize) -> Result<(), ExecutionError> {
    let next = used.checked_add(std::mem::size_of::<Number>())
        .ok_or_else(|| ExecutionError::Invalid("composite group median memory overflow".into()))?;
    if next > MAX_RETAINED_GROUP_BYTES {
        return Err(ExecutionError::Invalid(
            "composite GROUP BY median exceeds 64 MiB retained memory".into()
        ));
    }
    *used = next;
    Ok(())
}

fn charge_group(
    total: &mut usize,
    dimensions: usize,
    metrics: usize,
    payload_bytes: usize,
) -> Result<(), ExecutionError> {
    let per_key = std::mem::size_of::<Key>() + std::mem::size_of::<Cell>();
    let fixed = dimensions.checked_mul(per_key)
        .and_then(|keys| metrics.checked_mul(std::mem::size_of::<AggregateState>())
            .and_then(|states| keys.checked_add(states)))
        .ok_or_else(|| ExecutionError::Invalid("grouped metric memory overflow".into()))?;
    let used = payload_bytes.checked_mul(2)
        .and_then(|payload| fixed.checked_add(payload))
        .and_then(|bytes| total.checked_add(bytes))
        .ok_or_else(|| ExecutionError::Invalid("grouped metric memory overflow".into()))?;
    if used > MAX_RETAINED_GROUP_BYTES {
        return Err(ExecutionError::Invalid(
            "multi-key GROUP BY exceeds 64 MiB retained budget".into(),
        ));
    }
    *total = used;
    Ok(())
}


// Convert a non-nullable 8-bit numeric key into a compact slot index.
fn byte_group_key(column: &TypedColumn, row: usize) -> Option<u8> {
    match column {
        TypedColumn::Numeric(NumericColumn::UInt8(values)) => values.get(row).copied(),
        TypedColumn::Numeric(NumericColumn::Int8(values)) => {
            values.get(row).map(|value| *value as u8)
        }
        _ => None,
    }
}

fn numeric_value_at(column: &TypedColumn, row: usize) -> Result<Option<Number>, ExecutionError> {
    // Avoid constructing a generic Cell for every aggregate value in
    // large GROUP BY scans. NULLs are filtered directly from the null map.
    match column {
        TypedColumn::Numeric(values) => Ok(Some(super::number_at(values, row))),
        TypedColumn::NullableNumeric { values, nulls } => {
            if nulls[row] != 0 {
                Ok(None)
            } else {
                Ok(Some(super::number_at(values, row)))
            }
        }
        _ => Err(ExecutionError::Invalid(
            "numeric aggregates require numeric columns".into(),
        )),
    }
}

// Encode a composite GROUP BY key into reusable storage. String components
// borrow the column's decoded bytes; only the first occurrence of a group
// needs to own an encoded key. Tags and lengths preserve tuple boundaries.
pub(super) fn append_group_component(column: &TypedColumn, row: usize, buffer: &mut Vec<u8>) {
    if typed::is_null_at(column, row) {
        buffer.push(0);
        return;
    }
    match column {
        TypedColumn::Text(values) | TypedColumn::NullableText { values, .. } => {
            let bytes = values.get(row).expect("validated typed batch row");
            buffer.push(1);
            buffer.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            buffer.extend_from_slice(bytes);
        }
        TypedColumn::Uuid(values) => {
            buffer.push(2);
            buffer.extend_from_slice(values[row].as_bytes());
        }
        TypedColumn::Numeric(values) | TypedColumn::NullableNumeric { values, .. } => {
            match super::number_at(values, row) {
                Number::Unsigned(value) => {
                    buffer.push(3);
                    buffer.extend_from_slice(&value.to_le_bytes());
                }
                Number::Signed(value) => {
                    buffer.push(4);
                    buffer.extend_from_slice(&value.to_le_bytes());
                }
                Number::Float(value) => {
                    buffer.push(5);
                    // Match NumberKey: -0 and +0 are one group; all NaNs
                    // also share the same GROUP BY key.
                    let bits = if value == 0.0 {
                        0
                    } else if value.is_nan() {
                        f64::NAN.to_bits()
                    } else {
                        value.to_bits()
                    };
                    buffer.extend_from_slice(&bits.to_le_bytes());
                }
            }
        }
    }
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    columns: &[String],
    items: &[(AggregateFunction, String)],
    aliases: &[Option<String>],
    filters: &[Option<String>],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    for column in columns {
        if !types.contains_key(column) {
            return Err(ExecutionError::Invalid(format!(
                "unknown group column: {column}"
            )));
        }
    }

    let mut names = columns.to_vec();
    for (function, column) in items {
        let kind = types.get(column).ok_or_else(|| {
            ExecutionError::Invalid(format!("unknown aggregate column: {column}"))
        })?;
        if *function != AggregateFunction::Count
            && matches!(typed::base_type(kind), "String" | "UUID")
        {
            return Err(ExecutionError::Invalid(
                "numeric aggregates require numeric columns".into(),
            ));
        }
        if !names.contains(column) {
            names.push(column.clone());
        }
    }
    for filter in filters.iter().flatten() {
        if !names.contains(filter) {
            names.push(filter.clone());
        }
        let column_type = metadata.columns.iter().find(|column| &column.name == filter)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown filter column: {filter}")))?;
        if matches!(typed::base_type(&column_type.data_type), "String" | "UUID") {
            return Err(ExecutionError::Invalid("conditional metric filters must be numeric".into()));
        }
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let column = predicate.column();
        if !names.iter().any(|name| name == column) {
            names.push(column.to_owned());
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

    for (index, column) in columns.iter().enumerate() {
        if index != 0 {
            output.write_all(b"\t")?;
        }
        output.write_all(column.as_bytes())?;
    }
    for (index, (function, column)) in items.iter().enumerate() {
        if let Some(alias) = aliases.get(index).and_then(Option::as_deref) {
            write!(output, "\t{alias}")?;
        } else {
            if let Some(filter) = filters.get(index).and_then(Option::as_deref) {
                write!(output, "\t{}If({column}, {filter})", function.name())?;
            } else {
            write!(output, "\t{}({column})", function.name())?;
            }
        }
    }
    writeln!(output)?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let group_indices = columns
        .iter()
        .map(|column| indices[column])
        .collect::<Vec<_>>();
    let item_indices = items
        .iter()
        .map(|(_, column)| indices[column])
        .collect::<Vec<_>>();
    // Composite text GROUP BY otherwise clones every string key on each row.
    let has_text_key = columns.iter().any(|column| {
        matches!(typed::base_type(types[column].as_str()), "String")
    });
    let mut encoded_groups = HashMap::<Vec<u8>, usize>::new();
    let mut encoded_probe = Vec::<u8>::new();
    let filter_indices = filters.iter()
        .map(|value| value.as_ref().map(|column| indices[column]))
        .collect::<Vec<_>>();
    let mut by_key = HashMap::<Vec<Key>, usize>::new();
    let mut groups = Vec::<(Vec<Cell>, Vec<AggregateState>)>::new();
    let mut retained_group_bytes = 0usize;
    let mut retained_median_bytes = 0usize;
    let mut dense_slots = (columns.len() == 2
        && columns
            .iter()
            .all(|key| matches!(types.get(key).map(String::as_str), Some("UInt8" | "Int8"))))
    .then(|| vec![usize::MAX; 1 << 16]);
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    // Reuse a probe buffer for existing groups instead of allocating a Vec
    // of Cells and a Vec of Keys for every matching input row.
    let mut probe_key = Vec::<Key>::with_capacity(group_indices.len());
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let group = if let Some(slots) = dense_slots.as_mut() {
                let first = byte_group_key(&batch[group_indices[0]], row).ok_or_else(|| {
                    ExecutionError::Invalid("unexpected first byte GROUP BY type".into())
                })?;
                let second = byte_group_key(&batch[group_indices[1]], row).ok_or_else(|| {
                    ExecutionError::Invalid("unexpected second byte GROUP BY type".into())
                })?;
                let slot = (usize::from(first) << 8) | usize::from(second);
                let index = &mut slots[slot];
                if *index != usize::MAX {
                    *index
                } else {
                    charge_group(&mut retained_group_bytes, group_indices.len(), items.len(), 0)?;
                    *index = groups.len();
                    groups.push((
                        group_indices
                            .iter()
                            .map(|&column| typed::cell(&batch[column], row))
                            .collect(),
                        items
                            .iter()
                            .map(|(function, _)| AggregateState::new(*function))
                            .collect(),
                    ));
                    *index
                }
            } else if has_text_key {
                encoded_probe.clear();
                for &index in &group_indices {
                    append_group_component(&batch[index], row, &mut encoded_probe);
                }
                if let Some(&index) = encoded_groups.get(encoded_probe.as_slice()) {
                    index
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 200000 groups".into(),
                        ));
                    }
                    let index = groups.len();
                    let keys = group_indices
                        .iter()
                        .map(|&column| typed::cell(&batch[column], row))
                        .collect::<Vec<_>>();
                    charge_group(
                        &mut retained_group_bytes, group_indices.len(),
                        items.len(), encoded_probe.len(),
                    )?;
                    encoded_groups.insert(std::mem::take(&mut encoded_probe), index);
                    encoded_probe = Vec::with_capacity(group_indices.len() * 24);
                    groups.push((
                        keys,
                        items
                            .iter()
                            .map(|(function, _)| AggregateState::new(*function))
                            .collect(),
                    ));
                    index
                }
            } else {
                probe_key.clear();
                for &index in &group_indices {
                    probe_key.push(typed::key_at(&batch[index], row));
                }
                if let Some(&index) = by_key.get(probe_key.as_slice()) {
                    index
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 200000 groups".into(),
                        ));
                    }
                    let index = groups.len();
                    let keys = group_indices
                        .iter()
                        .map(|&column| typed::cell(&batch[column], row))
                        .collect::<Vec<_>>();
                    charge_group(&mut retained_group_bytes, group_indices.len(), items.len(), 0)?;
                    by_key.insert(std::mem::take(&mut probe_key), index);
                    probe_key = Vec::with_capacity(group_indices.len());
                    groups.push((
                        keys,
                        items
                            .iter()
                            .map(|(function, _)| AggregateState::new(*function))
                            .collect(),
                    ));
                    index
                }
            };
            for (metric, ((function, _), (&index, state))) in items
                .iter()
                .zip(item_indices.iter().zip(groups[group].1.iter_mut()))
                .enumerate()
            {
                if let Some(&filter_index) = filter_indices.get(metric).and_then(Option::as_ref) {
                    let enabled = match numeric_value_at(&batch[filter_index], row)? {
                        Some(Number::Unsigned(value)) => value != 0,
                        Some(Number::Signed(value)) => value != 0,
                        Some(Number::Float(value)) => value != 0.0,
                        None => false,
                    };
                    if !enabled {continue;}
                }
                if *function == AggregateFunction::Count {
                    if !typed::is_null_at(&batch[index], row) {
                        state.update(Number::Unsigned(1))?;
                    }
                    continue;
                }
                if let Some(value) = numeric_value_at(&batch[index], row)? {
                    if *function == AggregateFunction::Median {
                        charge_median_value(&mut retained_median_bytes)?;
                    }
                    state.update(value)?;
                }
            }
        }
    }

    if let Some(having) = &query.having {
        let index = having.aggregate_index.ok_or_else(|| {
            ExecutionError::Invalid("HAVING metric index is required".into())
        })?;
        groups.retain(|(_, states)| states.get(index)
            .is_some_and(|state| state.passes_having(having)));
    }
    let terms = query.order_terms().map(|term| {
        let metric = term.column.strip_prefix("#aggregate")
            .and_then(|number| number.parse::<usize>().ok());
        let group = columns.iter().position(|column| column == &term.column);
        (group, metric, term.descending, term.nulls_first)
    }).collect::<Vec<_>>();
    let compare = |(left_keys, left_states): &(Vec<Cell>, Vec<AggregateState>),
                   (right_keys, right_states): &(Vec<Cell>, Vec<AggregateState>)| {
        for &(group, metric, descending, nulls_first) in &terms {
            let order = if let Some(index) = metric {
                typed::compare_order_cells(&left_states[index].sort_cell(),
                    &right_states[index].sort_cell(), descending, nulls_first)
            } else {
                let index = group.expect("GROUP BY ORDER BY validated by parser");
                typed::compare_order_cells(&left_keys[index], &right_keys[index], descending, nulls_first)
            };
            if order != Ordering::Equal {
                return order;
            }
        }
        Ordering::Equal
    };
    let mut bounded_order = None;
    if !terms.is_empty() {
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

    // Resolve output schema once before serializing many grouped rows.
    let output_types = columns
        .iter()
        .map(|column| types[column].as_str())
        .collect::<Vec<_>>();
    let write_group =
        |keys: &[Cell], states: &[AggregateState], output: &mut W| -> Result<(), ExecutionError> {
            for (index, key) in keys.iter().enumerate() {
                if index != 0 {
                    output.write_all(b"\t")?;
                }
                typed::write_typed_value(output, key, output_types[index])?;
            }
            for state in states {
                write!(output, "\t{}", state.render())?;
            }
            writeln!(output)?;
            Ok(())
        };
    if let Some(indices) = bounded_order {
        for index in indices.into_iter().skip(query.offset.unwrap_or(0)) {
            write_group(&groups[index].0, &groups[index].1, output)?;
        }
    } else {
        for (keys, states) in groups
            .into_iter()
            .skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX))
        {
            write_group(&keys, &states, output)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod direct_grouped_numeric_tests {
    use super::*;

    #[test]
    fn compact_composite_keys_preserve_boundaries_nulls_and_float_normalization() {
        use rusthouse_storage::read_legacy_strings;
        let mut source = b"\x02ab\x01a\x02ab".as_slice();
        let strings = TypedColumn::Text(read_legacy_strings(&mut source, 3).unwrap());
        let numbers = TypedColumn::Numeric(NumericColumn::UInt64(vec![12, 12, 13]));
        let mut scratch = Vec::new();
        let key = |row, scratch: &mut Vec<u8>| {
            scratch.clear();
            append_group_component(&strings, row, scratch);
            append_group_component(&numbers, row, scratch);
            scratch.clone()
        };
        let first = key(0, &mut scratch);
        let second = key(1, &mut scratch);
        let third = key(2, &mut scratch);
        assert_ne!(first, second);
        assert_ne!(first, third);
        let zeroes = TypedColumn::Numeric(NumericColumn::Float64(vec![-0.0, 0.0, f64::NAN, f64::NAN]));
        scratch.clear();
        append_group_component(&zeroes, 0, &mut scratch);
        let positive_zero = scratch.clone();
        scratch.clear();
        append_group_component(&zeroes, 1, &mut scratch);
        assert_eq!(scratch, positive_zero);
        scratch.clear();
        append_group_component(&zeroes, 2, &mut scratch);
        let nan = scratch.clone();
        scratch.clear();
        append_group_component(&zeroes, 3, &mut scratch);
        assert_eq!(scratch, nan);
        let nulls = TypedColumn::NullableNumeric {
            values: NumericColumn::UInt64(vec![7, 7]),
            nulls: vec![1, 0],
        };
        scratch.clear();
        append_group_component(&nulls, 0, &mut scratch);
        assert_eq!(scratch, vec![0]);
        scratch.clear();
        append_group_component(&nulls, 1, &mut scratch);
        assert_ne!(scratch, vec![0]);
    }

    #[test]
    fn reads_grouped_aggregate_values_and_skips_nulls() {
        let plain = TypedColumn::Numeric(NumericColumn::UInt64(vec![7, 11]));
        assert!(matches!(
            numeric_value_at(&plain, 1).unwrap(),
            Some(Number::Unsigned(11))
        ));
        let nullable = TypedColumn::NullableNumeric {
            values: NumericColumn::Int16(vec![-8, 99]),
            nulls: vec![0, 1],
        };
        assert!(matches!(
            numeric_value_at(&nullable, 0).unwrap(),
            Some(Number::Signed(-8))
        ));
        assert!(numeric_value_at(&nullable, 1).unwrap().is_none());
    }
}

#[cfg(test)]
mod keyed_aggregate_group_memory_tests {
    use super::*;
    #[test]
    fn multi_dimension_budget_checks_overflow_before_retention() {
        let mut used = 0usize;
        charge_group(&mut used, 8, 6, 500).unwrap();
        let before = used;
        assert!(charge_group(&mut used, usize::MAX, 1, 0).is_err());
        assert_eq!(used, before);
        assert!(charge_group(&mut used, 8, 6, MAX_RETAINED_GROUP_BYTES).is_err());
    }
}

#[cfg(test)]
mod composite_group_median_retention_tests {
    use super::*;

    #[test]
    fn accounts_for_median_inputs_across_all_group_keys() {
        let mut used = 0usize;
        charge_median_value(&mut used).unwrap();
        let single = used;
        charge_median_value(&mut used).unwrap();
        assert_eq!(used, 2 * single);
        used = MAX_RETAINED_GROUP_BYTES - 1;
        assert!(charge_median_value(&mut used).is_err());
        assert_eq!(used, MAX_RETAINED_GROUP_BYTES - 1);
    }
}
