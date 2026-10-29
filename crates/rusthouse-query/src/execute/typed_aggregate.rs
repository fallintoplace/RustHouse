//! Numeric aggregates over typed wide parts, including text filters and nullable values.
use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError, Number};
use crate::{AggregateFunction, Projection, Select};
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, hash_map::Entry};
use std::io::Write;
use std::path::Path;

fn group_limit_error() -> ExecutionError {
    ExecutionError::Invalid("GROUP BY supports at most 100000 groups".into())
}

fn replaces_text_extreme(
    value: &[u8],
    current: Option<&Cell>,
    function: AggregateFunction,
) -> bool {
    let Some(Cell::Text(best)) = current else {
        return current.is_none();
    };
    match function {
        AggregateFunction::Min => value < best.as_slice(),
        AggregateFunction::Max => value > best.as_slice(),
        _ => false,
    }
}

pub(super) fn execute_typed_aggregate<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::Aggregate { function, column } = &query.projection else {
        unreachable!("typed aggregate requires an aggregate projection");
    };
    let metadata = inspect_query_source(part)?;
    let selected = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown aggregate column: {column}")))?;
    let ordered_text = matches!(typed::base_type(&selected.data_type), "String" | "UUID");
    if ordered_text && !matches!(function, AggregateFunction::Min | AggregateFunction::Max) {
        return Err(ExecutionError::Invalid(
            "numeric aggregates require a numeric column".into(),
        ));
    }

    let mut names = vec![column.clone()];
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !names.iter().any(|name| name == predicate.column()) {
            names.push(predicate.column().to_owned());
        }
    }
    let indices: HashMap<String, usize> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let types: HashMap<String, String> = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect();
    let first = typed::compile(&query.predicates, &indices, &types)?;
    let mut conditions = vec![first];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    let projected_index = indices[column];
    let header = format!("{}({column})", function.name());
    writeln!(output, "{}", query.alias.as_deref().unwrap_or(&header))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    if *function == AggregateFunction::Median {
        if let Some(inner) = selected.data_type.strip_prefix("Nullable(")
            .and_then(|name| name.strip_suffix(')'))
        {
            if super::radix_median::supports_type(inner) {
                let value = if no_filters {
                    super::radix_median::median_nullable(part, column, &metadata)?
                } else {
                    super::radix_median::median_nullable_filtered(
                        part, column, &metadata, &names, &conditions,
                    )?
                };
                if query.offset.unwrap_or(0) == 0 {
                    writeln!(output, "{}", value.unwrap_or_else(|| "NULL".into()))?;
                }
                return Ok(());
            }
        }
    }
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;

    let mut state = AggregateState::new(*function);
    let mut ordered_value: Option<Cell> = None;
    // Only stringify the winning UUID, not every scanned UUID row.
    let mut ordered_uuid: Option<[u8; 16]> = None;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            if ordered_text {
                match &batch[projected_index] {
                    TypedColumn::Text(strings)
                    | TypedColumn::NullableText {
                        values: strings, ..
                    } => {
                        if typed::is_null_at(&batch[projected_index], row) {
                            continue;
                        }
                        let value = strings.get(row).ok_or_else(|| {
                            ExecutionError::Invalid("missing String value in aggregate".into())
                        })?;
                        if replaces_text_extreme(value, ordered_value.as_ref(), *function) {
                            ordered_value = Some(Cell::Text(value.to_vec()));
                        }
                        continue;
                    }
                    TypedColumn::Uuid(values) => {
                        let value = *values[row].as_bytes();
                        let replace = ordered_uuid.is_none_or(|best| match function {
                            AggregateFunction::Min => value < best,
                            AggregateFunction::Max => value > best,
                            _ => false,
                        });
                        if replace {
                            ordered_uuid = Some(value);
                            ordered_value = Some(Cell::Uuid(values[row].to_string()));
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            // Avoid constructing Cell enums on the common numeric batch path.
            // NULLs remain skipped, and checked AggregateState math is shared.
            match &batch[projected_index] {
                TypedColumn::Numeric(values) => {
                    state.update(super::number_at(values, row))?;
                    continue;
                }
                TypedColumn::NullableNumeric { values, nulls } => {
                    if nulls[row] == 0 {
                        state.update(super::number_at(values, row))?;
                    }
                    continue;
                }
                _ => {}
            }
            match typed::cell(&batch[projected_index], row) {
                Cell::Numeric(value) => state.update(value)?,
                Cell::Null => {}
                value @ (Cell::Text(_) | Cell::Uuid(_)) if ordered_text => {
                    let replace = ordered_value.as_ref().is_none_or(|current| {
                        let ordering = typed::cmp(&value, current);
                        match function {
                            AggregateFunction::Min => ordering.is_lt(),
                            AggregateFunction::Max => ordering.is_gt(),
                            _ => false,
                        }
                    });
                    if replace {
                        ordered_value = Some(value);
                    }
                }
                Cell::Text(_) | Cell::Uuid(_) => {
                    return Err(ExecutionError::Invalid(
                        "numeric aggregates require a numeric column".into(),
                    ));
                }
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        if ordered_text {
            if let Some(value) = ordered_value {
                typed::write_typed_value(output, &value, &selected.data_type)?;
                writeln!(output)?;
            } else {
                writeln!(output, "NULL")?;
            }
            return Ok(());
        }
        let rendered = state.render();
        let rendered = match (function, selected.data_type.as_str()) {
            (AggregateFunction::Min | AggregateFunction::Max, "Date") => rendered
                .parse::<u16>()
                .map(rusthouse_storage::format_date)
                .unwrap_or(rendered),
            (AggregateFunction::Min | AggregateFunction::Max, "DateTime") => rendered
                .parse::<u32>()
                .map(rusthouse_storage::format_datetime)
                .unwrap_or(rendered),
            _ => rendered,
        };
        writeln!(output, "{rendered}")?;
    }
    Ok(())
}

fn update_group_numeric(
    state: &mut AggregateState,
    column: &TypedColumn,
    row: usize,
) -> Result<(), ExecutionError> {
    match column {
        TypedColumn::Numeric(values) => state.update(super::number_at(values, row)),
        TypedColumn::NullableNumeric { values, nulls } => {
            if nulls[row] == 0 {
                state.update(super::number_at(values, row))?;
            }
            Ok(())
        }
        _ => Err(ExecutionError::Invalid(
            "numeric aggregates require a numeric column".into(),
        )),
    }
}

const MAX_GROUPS: usize = 100_000;
const MAX_GROUPED_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// A new String GROUP BY key is owned by the lookup map and output Cell.
fn charge_grouped_text(used: &mut usize, text_len: usize) -> Result<(), ExecutionError> {
    let added = text_len.checked_mul(2)
        .and_then(|bytes| bytes.checked_add(
            std::mem::size_of::<(Vec<u8>, usize)>()
                + std::mem::size_of::<(Cell, AggregateState)>()
        ))
        .ok_or_else(|| ExecutionError::Invalid("GROUP BY String key bytes overflow".into()))?;
    let next = used.checked_add(added)
        .ok_or_else(|| ExecutionError::Invalid("GROUP BY String key bytes overflow".into()))?;
    if next > MAX_GROUPED_TEXT_BYTES {
        return Err(ExecutionError::Invalid(
            "GROUP BY String keys exceed 64 MiB retained memory".into()
        ));
    }
    *used = next;
    Ok(())
}
 
/// Encode a narrow integer dimension directly into its dense bucket.
/// Nullable dimensions reserve the final bucket for NULL rather than
/// hashing a new typed key on every qualifying analytical fact row.
fn narrow_group_bucket(
    column: &TypedColumn,
    row: usize,
    null_bucket: usize,
) -> Result<usize, ExecutionError> {
    let values = match column {
        TypedColumn::Numeric(values) => values,
        TypedColumn::NullableNumeric { values, nulls } => {
            if nulls[row] != 0 {
                return Ok(null_bucket);
            }
            values
        }
        _ => return Err(ExecutionError::Invalid("unexpected dense GROUP BY column".into())),
    };
    let bucket = match values {
        NumericColumn::UInt8(values) => usize::from(values[row]),
        NumericColumn::Int8(values) => usize::from(values[row] as u8),
        NumericColumn::UInt16(values) => usize::from(values[row]),
        NumericColumn::Int16(values) => usize::from(values[row] as u16),
        _ => return Err(ExecutionError::Invalid("unexpected dense GROUP BY column".into())),
    };
    Ok(bucket)
}



/// Exact integer SUM over parallel physical parts. Keep both the group key
/// and an empty aggregate state for keys whose measures are entirely NULL.
fn parallel_grouped_integer_sum(
    part: &Path,
    names: &[String],
    group_index: usize,
    value_index: usize,
    conditions: &[Vec<typed::Condition>],
    no_filters: bool,
) -> Result<Option<Vec<(Cell, AggregateState)>>, ExecutionError> {
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    let chunk_size = parts.len().div_ceil(workers);
    let chunks = std::thread::scope(|scope| {
        let handles = parts.chunks(chunk_size).map(|paths| {
            scope.spawn(move || -> Result<Vec<(Cell, AggregateState)>, ExecutionError> {
                let mut groups: Vec<(Cell, AggregateState)> = Vec::new();
                let mut by_key = HashMap::<typed::Key, usize>::new();
                for path in paths {
                    let mut reader = rusthouse_storage::TypedPartReader::open(path, names)?;
                    while let Some(batch) = reader.next_batch()? {
                        for row in 0..batch[0].len() {
                            if !no_filters && !typed::passes(&batch, row, conditions) {
                                continue;
                            }
                            let key = typed::key_at(&batch[group_index], row);
                            let group = match by_key.entry(key) {
                                Entry::Occupied(entry) => *entry.get(),
                                Entry::Vacant(entry) => {
                                    if groups.len() >= MAX_GROUPS {
                                        return Err(group_limit_error());
                                    }
                                    let id = groups.len();
                                    entry.insert(id);
                                    groups.push((
                                        typed::cell(&batch[group_index], row),
                                        AggregateState::new(AggregateFunction::Sum),
                                    ));
                                    id
                                }
                            };
                            update_group_numeric(
                                &mut groups[group].1, &batch[value_index], row,
                            )?;
                        }
                    }
                }
                Ok(groups)
            })
        }).collect::<Vec<_>>();
        handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| {
            Err(ExecutionError::Invalid("parallel grouped SUM worker panicked".into()))
        })).collect::<Vec<_>>()
    });
    let mut groups: Vec<(Cell, AggregateState)> = Vec::new();
    let mut by_key = HashMap::<typed::Key, usize>::new();
    for chunk in chunks {
        for (key, partial) in chunk? {
            let key_id = key.key();
            if let Some(&index) = by_key.get(&key_id) {
                if let Some(value) = partial.value {
                    groups[index].1.update(value)?;
                }
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(group_limit_error());
                }
                by_key.insert(key_id, groups.len());
                groups.push((key, partial));
            }
        }
    }
    Ok(Some(groups))
}

/// Group filtered rows by a typed key and evaluate a numeric aggregate.
pub(super) fn execute_grouped_aggregate<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::GroupedAggregate {
        column,
        function,
        value_column,
    } = &query.projection
    else {
        unreachable!("grouped executor requires a grouped aggregate")
    };
    let metadata = inspect_query_source(part)?;
    let selected = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *value_column)
        .ok_or_else(|| {
            ExecutionError::Invalid(format!("unknown aggregate column: {value_column}"))
        })?;
    if *function != AggregateFunction::Count
        && matches!(typed::base_type(&selected.data_type), "String" | "UUID")
    {
        return Err(ExecutionError::Invalid(
            "numeric aggregates require a numeric column".into(),
        ));
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
    let indices: HashMap<String, usize> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let types: HashMap<String, String> = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    let group_index = indices[column];
    let value_index = indices[value_column];
    let group_type = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .map(|candidate| candidate.data_type.as_str())
        .ok_or_else(|| ExecutionError::Invalid("unknown group column".into()))?;
    writeln!(output, "{column}\t{}({value_column})", function.name())?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let value_kind = types[value_column].as_str();
    let exact_integer = matches!(
        typed::base_type(value_kind),
        "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128"
            | "Int8" | "Int16" | "Int32" | "Int64" | "Int128"
    );
    let group_is_narrow = matches!(group_type, "UInt8" | "Int8" | "UInt16" | "Int16");
    let partial = if *function == AggregateFunction::Sum && exact_integer && group_is_narrow {
        parallel_grouped_integer_sum(
            part, &names, group_index, value_index, &conditions, no_filters,
        )?
    } else {
        None
    };
    let mut reader = if partial.is_some() {
        None
    } else {
        Some(TypedQueryReader::open_with_metadata(part, &names, &metadata)?)
    };

    let mut by_key = HashMap::<typed::Key, usize>::new();
    // Keep String keys in an owned map and probe with borrowed bytes.
    // Repeated station names no longer allocate a new key on every row.
    let mut text_groups = HashMap::<Vec<u8>, usize>::new();
    let mut retained_text_bytes = 0usize;
    let mut uuid_groups = HashMap::<[u8; 16], usize>::new();
    let mut groups = partial.unwrap_or_default();
    // Direct numeric buckets also include nullable dimensions. Reserve a
    // separate sentinel bucket so NULL never aliases integer zero.
    let mut dense_slots = match group_type {
        "UInt8" | "Int8" => Some(vec![usize::MAX; 256]),
        "UInt16" | "Int16" => Some(vec![usize::MAX; 65_536]),
        "Nullable(UInt8)" | "Nullable(Int8)" => Some(vec![usize::MAX; 257]),
        "Nullable(UInt16)" | "Nullable(Int16)" => Some(vec![usize::MAX; 65_537]),
        _ => None,
    };
    while let Some(batch) = reader.as_mut().map_or(Ok(None), TypedQueryReader::next_batch)? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let group = if let Some(slots) = dense_slots.as_mut() {
                let key = narrow_group_bucket(
                    &batch[group_index],
                    row,
                    slots.len() - 1,
                )?;
                let group = &mut slots[key];
                if *group != usize::MAX {
                    *group
                } else {
                    let index = groups.len();
                    *group = index;
                    groups.push((
                        typed::cell(&batch[group_index], row),
                        AggregateState::new(*function),
                    ));
                    index
                }
            } else {
                let borrowed_text = match &batch[group_index] {
                    TypedColumn::Text(strings) => strings.get(row),
                    TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
                        values.get(row)
                    }
                    _ => None,
                };
                let borrowed_uuid = match &batch[group_index] {
                    TypedColumn::Uuid(values) => Some(*values[row].as_bytes()),
                    _ => None,
                };
                if let Some(key) = borrowed_uuid {
                    if let Some(&existing) = uuid_groups.get(&key) {
                        existing
                    } else {
                        if groups.len() >= MAX_GROUPS {
                            return Err(group_limit_error());
                        }
                        let index = groups.len();
                        uuid_groups.insert(key, index);
                        groups.push((
                            typed::cell(&batch[group_index], row),
                            AggregateState::new(*function),
                        ));
                        index
                    }
                } else if let Some(key) = borrowed_text {
                    if let Some(&existing) = text_groups.get(key) {
                        existing
                    } else {
                        if groups.len() >= MAX_GROUPS {
                            return Err(group_limit_error());
                        }
                        charge_grouped_text(&mut retained_text_bytes, key.len())?;
                        let index = groups.len();
                        text_groups.insert(key.to_vec(), index);
                        groups.push((Cell::Text(key.to_vec()), AggregateState::new(*function)));
                        index
                    }
                } else {
                    let key = typed::cell(&batch[group_index], row);
                    match by_key.entry(key.key()) {
                        Entry::Occupied(entry) => *entry.get(),
                        Entry::Vacant(entry) => {
                            if groups.len() >= MAX_GROUPS {
                                return Err(group_limit_error());
                            }
                            entry.insert(groups.len());
                            groups.push((key, AggregateState::new(*function)));
                            groups.len() - 1
                        }
                    }
                }
            };
            if *function == AggregateFunction::Count {
                if !typed::is_null_at(&batch[value_index], row) {
                    groups[group].1.increment_count()?;
                }
                continue;
            }
            // Numeric aggregate inputs are already native column vectors.
            // Do not construct a Cell for each GROUP BY input row.
            update_group_numeric(&mut groups[group].1, &batch[value_index], row)?;
        }
    }
    if let Some(having) = &query.having {
        groups.retain(|(_, state)| state.passes_having(having));
    }
    let mut bounded_order = None;
    if query.order_by.is_some() {
        // Resolve the small ORDER BY specification once. The comparator can
        // run O(groups * log groups) times for a large grouped result.
        let order_terms = query
            .order_terms()
            .map(|term| {
                (term.column == "#aggregate", term.descending, term.nulls_first)
            })
            .collect::<Vec<_>>();
        let compare = |(left_key, left_state): &(Cell, AggregateState),
                       (right_key, right_state): &(Cell, AggregateState)| {
            for &(aggregate, descending, nulls_first) in &order_terms {
                let comparison = if aggregate {
                    typed::compare_order_cells(
                        &left_state.sort_cell(),
                        &right_state.sort_cell(),
                        descending,
                        nulls_first,
                    )
                } else {
                    typed::compare_order_cells(
                        left_key,
                        right_key,
                        descending,
                        nulls_first,
                    )
                };
                if comparison != std::cmp::Ordering::Equal {
                    return comparison;
                }
            }
            std::cmp::Ordering::Equal
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
    let write_group =
        |key: &Cell, state: &AggregateState, output: &mut W| -> Result<(), ExecutionError> {
            typed::write_typed_value(output, key, group_type)?;
            writeln!(output, "\t{}", state.render())?;
            Ok(())
        };
    if let Some(indices) = bounded_order {
        for index in indices.into_iter().skip(query.offset.unwrap_or(0)) {
            write_group(&groups[index].0, &groups[index].1, output)?;
        }
    } else {
        for (key, state) in groups
            .iter()
            .skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX))
        {
            write_group(key, state, output)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod borrowed_text_extrema_tests {
    use super::*;

    #[test]
    fn compares_borrowed_text_without_replacing_ties() {
        let incumbent = Cell::Text(b"beta".to_vec());
        assert!(replaces_text_extreme(
            b"alpha",
            Some(&incumbent),
            AggregateFunction::Min
        ));
        assert!(!replaces_text_extreme(
            b"beta",
            Some(&incumbent),
            AggregateFunction::Min
        ));
        assert!(!replaces_text_extreme(
            b"alpha",
            Some(&incumbent),
            AggregateFunction::Max
        ));
        assert!(replaces_text_extreme(
            b"gamma",
            Some(&incumbent),
            AggregateFunction::Max
        ));
        assert!(replaces_text_extreme(b"beta", None, AggregateFunction::Min));
    }
}

#[cfg(test)]
mod grouped_direct_numeric_aggregate_tests {
    use super::*;

    #[test]
    fn direct_grouped_sum_skips_nulls_and_preserves_signed_values() {
        let column = TypedColumn::NullableNumeric {
            values: NumericColumn::Int64(vec![-3, 17, 9]),
            nulls: vec![0, 1, 0],
        };
        let mut aggregate = AggregateState::new(AggregateFunction::Sum);
        for row in 0..column.len() {
            update_group_numeric(&mut aggregate, &column, row).unwrap();
        }
        assert_eq!(aggregate.render(), "6");
        let non_numeric = TypedColumn::Numeric(NumericColumn::UInt64(vec![4]));
        update_group_numeric(&mut aggregate, &non_numeric, 0).unwrap();
        assert_eq!(aggregate.render(), "10");
    }
}

#[cfg(test)]
mod nullable_numeric_aggregate_bucket_tests {
    use super::*;

    #[test]
    fn null_is_separate_from_zero_and_all_signed_boundary_keys() {
        let nullable = TypedColumn::NullableNumeric {
            values: NumericColumn::Int8(vec![0, -1, i8::MIN, i8::MAX, 0]),
            nulls: vec![1, 0, 0, 0, 0],
        };
        assert_eq!(
            (0..5).map(|index| narrow_group_bucket(&nullable, index, 256).unwrap()).collect::<Vec<_>>(),
            [256, 255, 128, 127, 0],
        );
        let nullable_16 = TypedColumn::NullableNumeric {
            values: NumericColumn::UInt16(vec![0, 0, u16::MAX, 42]),
            nulls: vec![0, 1, 0, 0],
        };
        assert_eq!(
            (0..4).map(|index| narrow_group_bucket(&nullable_16, index, 65_536).unwrap()).collect::<Vec<_>>(),
            [0, 65_536, 65_535, 42],
        );
    }

    #[test]
    fn same_dense_bucket_reuses_group_state_for_nullable_sum() {
        let dimension = TypedColumn::NullableNumeric {
            values: NumericColumn::UInt8(vec![1, 1, 2, 0, 0, 1]),
            nulls: vec![0, 0, 0, 1, 1, 0],
        };
        let amounts = TypedColumn::Numeric(NumericColumn::UInt64(vec![10, 20, 30, 40, 50, 60]));
        let mut slots = vec![usize::MAX; 257];
        let mut groups = Vec::<(Cell, AggregateState)>::new();
        for row in 0..6 {
            let bucket = narrow_group_bucket(&dimension, row, 256).unwrap();
            if slots[bucket] == usize::MAX {
                slots[bucket] = groups.len();
                groups.push((typed::cell(&dimension, row), AggregateState::new(AggregateFunction::Sum)));
            }
            update_group_numeric(&mut groups[slots[bucket]].1, &amounts, row).unwrap();
        }
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[slots[1]].1.render(), "90");
        assert_eq!(groups[slots[2]].1.render(), "30");
        assert_eq!(groups[slots[256]].1.render(), "90");
        assert!(matches!(groups[slots[256]].0, Cell::Null));
    }
}

#[cfg(test)]
mod grouped_text_budget_tests {
    use super::*;
    #[test]
    fn repeated_text_keys_are_only_charged_once_by_the_caller() {
        let mut used = 0usize;
        charge_grouped_text(&mut used, 4096).unwrap();
        assert!(used >= 8192);
        let previous = used;
        assert!(charge_grouped_text(&mut used, MAX_GROUPED_TEXT_BYTES).is_err());
        assert_eq!(used, previous);
        assert!(charge_grouped_text(&mut used, usize::MAX).is_err());
    }
}
