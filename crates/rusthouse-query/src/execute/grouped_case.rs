//! Apply a conditional CASE before retaining grouped key values.
use super::*;
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
const MAX_GROUPS: usize = 200_000;
const MAX_GROUP_BYTES: usize = 64 * 1024 * 1024;
const GROUP_MEMORY_OVERHEAD: usize = 192;

fn charge_group_bytes(
    retained: &mut usize,
    encoded_capacity: usize,
    output_capacity: usize,
) -> Result<(), ExecutionError> {
    let bytes = GROUP_MEMORY_OVERHEAD
        .checked_add(encoded_capacity)
        .and_then(|total| total.checked_add(output_capacity))
        .ok_or_else(|| ExecutionError::Invalid("CASE group memory overflow".into()))?;
    *retained = retained
        .checked_add(bytes)
        .filter(|&total| total <= MAX_GROUP_BYTES)
        .ok_or_else(|| ExecutionError::Invalid("CASE groups exceed 64 MiB".into()))?;
    Ok(())
}

fn charge_group_memory(
    retained: &mut usize,
    encoded_capacity: usize,
    output_key: &[Vec<u8>; 5],
) -> Result<(), ExecutionError> {
    let output_capacity = output_key
        .iter()
        .try_fold(0usize, |total, value| total.checked_add(value.capacity()));
    let output_capacity = output_capacity
        .ok_or_else(|| ExecutionError::Invalid("CASE group memory overflow".into()))?;
    charge_group_bytes(retained, encoded_capacity, output_capacity)
}

fn integer_zero(column: &TypedColumn, row: usize) -> bool {
    let values = match column {
        TypedColumn::Numeric(values) => values,
        TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 => values,
        TypedColumn::NullableNumeric { .. } => return false,
        _ => return false,
    };
    matches!(
        super::number_at(values, row),
        super::Number::Signed(0) | super::Number::Unsigned(0)
    )
}

fn append_case_integer_component(column: &TypedColumn, row: usize, buffer: &mut Vec<u8>) {
    let (values, is_null) = match column {
        TypedColumn::Numeric(values) => (values, false),
        TypedColumn::NullableNumeric { values, nulls } => (values, nulls[row] != 0),
        _ => unreachable!("CASE integer key was validated before scanning"),
    };
    buffer.push(u8::from(!is_null));
    match values {
        rusthouse_storage::NumericColumn::UInt8(values) => {
            buffer.push(if is_null { 0 } else { values[row] });
        }
        rusthouse_storage::NumericColumn::Int8(values) => {
            buffer.push(if is_null { 0 } else { values[row] as u8 });
        }
        rusthouse_storage::NumericColumn::UInt16(values) => {
            buffer.extend_from_slice(&if is_null { 0 } else { values[row] }.to_le_bytes());
        }
        rusthouse_storage::NumericColumn::Int16(values) => {
            buffer.extend_from_slice(&if is_null { 0 } else { values[row] }.to_le_bytes());
        }
        rusthouse_storage::NumericColumn::UInt32(values) => {
            buffer.extend_from_slice(&if is_null { 0 } else { values[row] }.to_le_bytes());
        }
        rusthouse_storage::NumericColumn::Int32(values) => {
            buffer.extend_from_slice(&if is_null { 0 } else { values[row] }.to_le_bytes());
        }
        rusthouse_storage::NumericColumn::UInt64(values) => {
            buffer.extend_from_slice(&if is_null { 0 } else { values[row] }.to_le_bytes());
        }
        rusthouse_storage::NumericColumn::Int64(values) => {
            buffer.extend_from_slice(&if is_null { 0 } else { values[row] }.to_le_bytes());
        }
        rusthouse_storage::NumericColumn::UInt128(values) => {
            buffer.extend_from_slice(&if is_null { 0 } else { values[row] }.to_le_bytes());
        }
        rusthouse_storage::NumericColumn::Int128(values) => {
            buffer.extend_from_slice(&if is_null { 0 } else { values[row] }.to_le_bytes());
        }
        rusthouse_storage::NumericColumn::Float32(_)
        | rusthouse_storage::NumericColumn::Float64(_) => {
            unreachable!("CASE integer key was validated before scanning")
        }
    }
}

fn output_component(column: &TypedColumn, row: usize) -> Result<Vec<u8>, ExecutionError> {
    let mut output = Vec::new();
    typed::write_value(&mut output, &typed::cell(column, row))?;
    Ok(output)
}

fn output_group_key(
    batch: &[TypedColumn],
    group_indices: &[usize],
    row: usize,
    use_source: bool,
) -> Result<[Vec<u8>; 5], ExecutionError> {
    Ok([
        output_component(&batch[group_indices[0]], row)?,
        output_component(&batch[group_indices[1]], row)?,
        output_component(&batch[group_indices[2]], row)?,
        if use_source {
            output_component(&batch[group_indices[3]], row)?
        } else {
            Vec::new()
        },
        output_component(&batch[group_indices[4]], row)?,
    ])
}

fn append_empty_text_component(buffer: &mut Vec<u8>) {
    buffer.push(1);
    buffer.extend_from_slice(&0u64.to_le_bytes());
}

fn integer(kind: &str) -> bool {
    matches!(
        typed::base_type(kind),
        "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
    )
}

// Group keys are unique, so a bounded result needs only the prefix ending
// at OFFSET + LIMIT rather than a full sort of every group.
fn sort_group_page<K: Ord>(
    rows: &mut Vec<(K, u64)>,
    descending: bool,
    offset: usize,
    limit: Option<usize>,
) {
    if limit == Some(0) || offset >= rows.len() {
        rows.clear();
        return;
    }
    let compare = |a: &(K, u64), b: &(K, u64)| {
        let cmp = if descending {
            b.1.cmp(&a.1)
        } else {
            a.1.cmp(&b.1)
        };
        cmp.then_with(|| a.0.cmp(&b.0))
    };
    let page_end = offset
        .saturating_add(limit.unwrap_or(usize::MAX))
        .min(rows.len());
    if page_end < rows.len() {
        rows.select_nth_unstable_by(page_end - 1, &compare);
        rows.truncate(page_end);
    }
    rows.sort_unstable_by(compare);
}

pub(super) fn execute<W: Write>(
    query: &Select,
    path: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let case = query
        .grouped_case_key
        .as_ref()
        .ok_or_else(|| ExecutionError::Invalid("CASE grouping plan is missing".into()))?;
    let path = path
        .ok_or_else(|| ExecutionError::Invalid("CASE grouping requires a stored table".into()))?;
    let metadata = inspect_query_source(path)?;
    if metadata.columns.iter().any(|column| {
        column.name.eq_ignore_ascii_case(&case.case_label)
            || column.name.eq_ignore_ascii_case(&case.destination_label)
    }) {
        return Err(ExecutionError::Invalid(
            "CASE alias conflicts with a source column".into(),
        ));
    }
    let (keys, count_label) = match &query.projection {
        Projection::GroupedMultiCount { columns } => (
            columns.as_slice(),
            query.alias.as_deref().unwrap_or("count"),
        ),
        Projection::GroupedMultiAggregates {
            columns,
            items,
            aliases,
            ..
        } if matches!(items.as_slice(), [(AggregateFunction::Count, value)] if value == "*") => (
            columns.as_slice(),
            aliases
                .first()
                .and_then(|alias| alias.as_deref())
                .or(query.alias.as_deref())
                .unwrap_or("count"),
        ),
        _ => {
            return Err(ExecutionError::Invalid(
                "CASE grouping requires one COUNT and five keys".into(),
            ));
        }
    };
    if keys.len() != 5 {
        return Err(ExecutionError::Invalid(
            "CASE grouping requires five keys".into(),
        ));
    }
    for (index, name) in keys.iter().enumerate() {
        let kind = metadata
            .columns
            .iter()
            .find(|column| &column.name == name)
            .ok_or_else(|| ExecutionError::Invalid("unknown CASE group key".into()))?;
        if index < 3 && !integer(&kind.data_type) {
            return Err(ExecutionError::Invalid(
                "CASE grouping requires integer keys".into(),
            ));
        }
        if index >= 3 && typed::base_type(&kind.data_type) != "String" {
            return Err(ExecutionError::Invalid(
                "CASE grouping requires String keys".into(),
            ));
        }
    }
    if keys[1] != case.first_flag || keys[2] != case.second_flag {
        return Err(ExecutionError::Invalid("CASE grouping flags differ".into()));
    }
    let mut names = keys.to_vec();
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
    let conditions = std::iter::once(query.predicates.as_slice())
        .chain(query.or_predicates.iter().map(Vec::as_slice))
        .map(|predicates| typed::compile(predicates, &indices, &types))
        .collect::<Result<Vec<_>, _>>()?;
    let group_indices = keys.iter().map(|name| indices[name]).collect::<Vec<_>>();
    if query.limit == Some(0) {
        write_case_header(output, keys, case, count_label)?;
        return Ok(());
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let mut reader = TypedQueryReader::open_with_metadata(path, &names, &metadata)?;
    let mut encoded_groups = HashMap::<Vec<u8>, usize>::new();
    let mut encoded_probe = Vec::with_capacity(128);
    let mut groups = Vec::<([Vec<u8>; 5], u64)>::new();
    let mut retained = 0usize;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            // Collapse the unused referer before retaining a group key.
            let use_source = integer_zero(&batch[group_indices[1]], row)
                && integer_zero(&batch[group_indices[2]], row);
            encoded_probe.clear();
            for &index in &group_indices[..3] {
                append_case_integer_component(&batch[index], row, &mut encoded_probe);
            }
            if use_source {
                super::grouped_numeric_keys::append_group_component(
                    &batch[group_indices[3]],
                    row,
                    &mut encoded_probe,
                );
            } else {
                append_empty_text_component(&mut encoded_probe);
            }
            super::grouped_numeric_keys::append_group_component(
                &batch[group_indices[4]],
                row,
                &mut encoded_probe,
            );

            if let Some(&index) = encoded_groups.get(encoded_probe.as_slice()) {
                groups[index].1 = groups[index].1.checked_add(1).ok_or_else(|| {
                    ExecutionError::Invalid("CASE grouping count overflow".into())
                })?;
                continue;
            }
            if groups.len() >= MAX_GROUPS {
                return Err(ExecutionError::Invalid(
                    "CASE grouping exceeds 200000 groups".into(),
                ));
            }
            let key = output_group_key(&batch, &group_indices, row, use_source)?;
            let encoded_key = encoded_probe.clone();
            charge_group_memory(&mut retained, encoded_key.capacity(), &key)?;
            let index = groups.len();
            encoded_groups.insert(encoded_key, index);
            groups.push((key, 1));
        }
    }
    let mut rows = groups;
    let descending = query
        .order_by
        .as_ref()
        .is_some_and(|order| order.descending);
    sort_group_page(
        &mut rows,
        descending,
        query.offset.unwrap_or(0),
        query.limit,
    );
    write_case_header(output, keys, case, count_label)?;
    for (keys, count) in rows
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                output.write_all(b"\t")?;
            }
            output.write_all(key)?;
        }
        writeln!(output, "\t{count}")?;
    }
    Ok(())
}

fn write_case_header<W: Write>(
    output: &mut W,
    keys: &[String],
    case: &crate::GroupedCaseKey,
    count_label: &str,
) -> Result<(), ExecutionError> {
    for (index, name) in keys.iter().take(3).enumerate() {
        if index > 0 {
            output.write_all(b"\t")?;
        }
        output.write_all(name.as_bytes())?;
    }
    writeln!(
        output,
        "\t{}\t{}\t{}",
        case.case_label, case.destination_label, count_label
    )?;
    Ok(())
}

#[cfg(test)]
mod topk_tests {
    use super::sort_group_page;

    #[test]
    fn bounded_case_group_pages_match_full_sort_with_ties() {
        let groups = (0..128u32)
            .map(|key| (key, u64::from(key % 9)))
            .collect::<Vec<_>>();
        for descending in [false, true] {
            for (offset, limit) in [
                (0, Some(1)),
                (5, Some(11)),
                (120, Some(20)),
                (128, None),
                (0, None),
                (0, Some(0)),
            ] {
                let mut expected = groups.clone();
                expected.sort_by(|a, b| {
                    let cmp = if descending {
                        b.1.cmp(&a.1)
                    } else {
                        a.1.cmp(&b.1)
                    };
                    cmp.then_with(|| a.0.cmp(&b.0))
                });
                let mut actual = groups.clone();
                sort_group_page(&mut actual, descending, offset, limit);
                assert_eq!(
                    actual
                        .into_iter()
                        .skip(offset)
                        .take(limit.unwrap_or(usize::MAX))
                        .collect::<Vec<_>>(),
                    expected
                        .into_iter()
                        .skip(offset)
                        .take(limit.unwrap_or(usize::MAX))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

#[cfg(test)]
mod case_memory_tests {
    use super::*;

    #[test]
    fn retained_budget_counts_both_stored_key_buffers_and_checks_overflow() {
        let keys = std::array::from_fn::<_, 5, _>(|_| vec![b'x'; 8]);
        let mut retained = 0;
        charge_group_memory(&mut retained, 24, &keys).unwrap();
        assert_eq!(retained, GROUP_MEMORY_OVERHEAD + 24 + 5 * 8);

        let before = retained;
        assert!(charge_group_bytes(&mut retained, usize::MAX, 0).is_err());
        assert_eq!(retained, before);

        let mut at_limit = 0;
        charge_group_bytes(&mut at_limit, MAX_GROUP_BYTES - GROUP_MEMORY_OVERHEAD, 0).unwrap();
        assert_eq!(at_limit, MAX_GROUP_BYTES);
        assert!(charge_group_bytes(&mut at_limit, 0, 0).is_err());
        assert_eq!(at_limit, MAX_GROUP_BYTES);
    }

    #[test]
    fn null_case_flags_do_not_select_the_source_string() {
        use rusthouse_storage::NumericColumn;

        let flags = TypedColumn::NullableNumeric {
            values: NumericColumn::Int16(vec![0, 0, 1]),
            nulls: vec![0, 1, 0],
        };
        assert!(integer_zero(&flags, 0));
        assert!(!integer_zero(&flags, 1));
        assert!(!integer_zero(&flags, 2));
    }

    #[test]
    fn compact_integer_components_preserve_null_and_signed_values() {
        use rusthouse_storage::NumericColumn;

        let values = TypedColumn::NullableNumeric {
            values: NumericColumn::Int16(vec![0, i16::MIN, 0]),
            nulls: vec![0, 0, 1],
        };
        let mut zero = Vec::new();
        append_case_integer_component(&values, 0, &mut zero);
        let mut minimum = Vec::new();
        append_case_integer_component(&values, 1, &mut minimum);
        let mut null = Vec::new();
        append_case_integer_component(&values, 2, &mut null);

        assert_eq!(zero.len(), 3);
        assert_eq!(minimum.len(), 3);
        assert_eq!(null.len(), 3);
        assert_ne!(zero, minimum);
        assert_ne!(zero, null);
        assert_eq!(minimum, [vec![1], i16::MIN.to_le_bytes().to_vec()].concat());
        assert_eq!(null, [vec![0], vec![0, 0]].concat());
    }

    #[test]
    fn compact_integer_components_keep_fixed_width_for_wide_unsigned_values() {
        use rusthouse_storage::NumericColumn;

        let values = TypedColumn::Numeric(NumericColumn::UInt128(vec![0, u128::MAX]));
        let mut zero = Vec::new();
        append_case_integer_component(&values, 0, &mut zero);
        let mut maximum = Vec::new();
        append_case_integer_component(&values, 1, &mut maximum);

        assert_eq!(zero.len(), 17);
        assert_eq!(maximum.len(), 17);
        assert_ne!(zero, maximum);
        assert_eq!(
            maximum,
            [vec![1], u128::MAX.to_le_bytes().to_vec()].concat()
        );
    }

    #[test]
    fn empty_case_component_matches_an_empty_string_key() {
        let mut actual = Vec::new();
        append_empty_text_component(&mut actual);
        assert_eq!(actual, [vec![1], vec![0; 8]].concat());
    }
}
