//! Apply a conditional CASE before retaining grouped key values.
use super::*;
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
const MAX_GROUPS: usize = 200_000;
const MAX_GROUP_BYTES: usize = 64 * 1024 * 1024;
const GROUP_MEMORY_OVERHEAD: usize = 192;

fn charge_group_bytes(
    retained: &mut usize,
    group_key_capacity: usize,
) -> Result<(), ExecutionError> {
    let bytes = GROUP_MEMORY_OVERHEAD
        .checked_add(group_key_capacity)
        .ok_or_else(|| ExecutionError::Invalid("CASE group memory overflow".into()))?;
    *retained = retained
        .checked_add(bytes)
        .filter(|&total| total <= MAX_GROUP_BYTES)
        .ok_or_else(|| ExecutionError::Invalid("CASE groups exceed 64 MiB".into()))?;
    Ok(())
}

fn charge_group_memory(
    retained: &mut usize,
    group_key_capacity: usize,
) -> Result<(), ExecutionError> {
    charge_group_bytes(retained, group_key_capacity)
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

fn append_case_integer_component<W: Write>(
    column: &TypedColumn,
    row: usize,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let values = match column {
        TypedColumn::Numeric(values) => values,
        TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 => values,
        TypedColumn::NullableNumeric { .. } => {
            output.write_all(b"\\N")?;
            return Ok(());
        }
        _ => {
            return Err(ExecutionError::Invalid(
                "CASE grouping requires integer keys".into(),
            ));
        }
    };
    write!(output, "{}", super::number_at(values, row))?;
    Ok(())
}

fn append_case_text_component<W: Write>(
    column: &TypedColumn,
    row: usize,
    output: &mut W,
) -> Result<(), ExecutionError> {
    match column {
        TypedColumn::Text(values) => {
            typed::write_text_bytes(output, values.get(row).expect("validated typed batch row"))?;
        }
        TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
            typed::write_text_bytes(output, values.get(row).expect("validated typed batch row"))?;
        }
        TypedColumn::NullableText { .. } => output.write_all(b"\\N")?,
        _ => {
            return Err(ExecutionError::Invalid(
                "CASE grouping requires String keys".into(),
            ));
        }
    }
    Ok(())
}

// Escaped text cannot contain a literal tab, so this joined key keeps the
// previous field-by-field tie ordering while storing only one output buffer.
fn append_case_group_key(
    batch: &[TypedColumn],
    group_indices: &[usize],
    row: usize,
    use_source: bool,
    key: &mut Vec<u8>,
) -> Result<(), ExecutionError> {
    key.clear();
    for (index, &column) in group_indices[..3].iter().enumerate() {
        if index > 0 {
            key.push(b'\t');
        }
        append_case_integer_component(&batch[column], row, key)?;
    }
    key.push(b'\t');
    if use_source {
        append_case_text_component(&batch[group_indices[3]], row, key)?;
    }
    key.push(b'\t');
    append_case_text_component(&batch[group_indices[4]], row, key)?;
    Ok(())
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
    let mut groups = HashMap::<Vec<u8>, u64>::new();
    let mut group_key_probe = Vec::with_capacity(128);
    let mut retained = 0usize;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            // Collapse the unused referer before retaining a group key.
            let use_source = integer_zero(&batch[group_indices[1]], row)
                && integer_zero(&batch[group_indices[2]], row);
            append_case_group_key(
                &batch,
                &group_indices,
                row,
                use_source,
                &mut group_key_probe,
            )?;

            if let Some(count) = groups.get_mut(group_key_probe.as_slice()) {
                *count = count.checked_add(1).ok_or_else(|| {
                    ExecutionError::Invalid("CASE grouping count overflow".into())
                })?;
                continue;
            }
            if groups.len() >= MAX_GROUPS {
                return Err(ExecutionError::Invalid(
                    "CASE grouping exceeds 200000 groups".into(),
                ));
            }
            let group_key = group_key_probe.clone();
            charge_group_memory(&mut retained, group_key.capacity())?;
            groups.insert(group_key, 1);
        }
    }
    let mut rows = groups.into_iter().collect::<Vec<_>>();
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
    for (key, count) in rows
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        output.write_all(&key)?;
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
    fn retained_budget_counts_one_stored_key_buffer_and_checks_overflow() {
        let mut retained = 0;
        charge_group_memory(&mut retained, 24).unwrap();
        assert_eq!(retained, GROUP_MEMORY_OVERHEAD + 24);

        let before = retained;
        assert!(charge_group_bytes(&mut retained, usize::MAX).is_err());
        assert_eq!(retained, before);

        let mut at_limit = 0;
        charge_group_bytes(&mut at_limit, MAX_GROUP_BYTES - GROUP_MEMORY_OVERHEAD).unwrap();
        assert_eq!(at_limit, MAX_GROUP_BYTES);
        assert!(charge_group_bytes(&mut at_limit, 0).is_err());
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
    fn integer_group_keys_format_all_widths_and_preserve_large_values() {
        use rusthouse_storage::NumericColumn;

        let samples = [
            (
                "UInt8",
                TypedColumn::Numeric(NumericColumn::UInt8(vec![u8::MAX])),
                "255",
            ),
            (
                "Int8",
                TypedColumn::Numeric(NumericColumn::Int8(vec![i8::MIN])),
                "-128",
            ),
            (
                "UInt16",
                TypedColumn::Numeric(NumericColumn::UInt16(vec![u16::MAX])),
                "65535",
            ),
            (
                "Int16",
                TypedColumn::Numeric(NumericColumn::Int16(vec![i16::MIN])),
                "-32768",
            ),
            (
                "UInt32",
                TypedColumn::Numeric(NumericColumn::UInt32(vec![u32::MAX])),
                "4294967295",
            ),
            (
                "Int32",
                TypedColumn::Numeric(NumericColumn::Int32(vec![i32::MIN])),
                "-2147483648",
            ),
            (
                "UInt64",
                TypedColumn::Numeric(NumericColumn::UInt64(vec![9_007_199_254_740_993])),
                "9007199254740993",
            ),
            (
                "Int64",
                TypedColumn::Numeric(NumericColumn::Int64(vec![i64::MIN])),
                "-9223372036854775808",
            ),
            (
                "UInt128",
                TypedColumn::Numeric(NumericColumn::UInt128(vec![u128::MAX])),
                "340282366920938463463374607431768211455",
            ),
            (
                "Int128",
                TypedColumn::Numeric(NumericColumn::Int128(vec![i128::MIN])),
                "-170141183460469231731687303715884105728",
            ),
            (
                "Nullable(Int16)",
                TypedColumn::NullableNumeric {
                    values: NumericColumn::Int16(vec![0]),
                    nulls: vec![1],
                },
                "\\N",
            ),
        ];

        for (data_type, column, expected) in samples {
            let mut actual = Vec::new();
            append_case_integer_component(&column, 0, &mut actual).unwrap();
            assert_eq!(actual, expected.as_bytes(), "{data_type}");
        }
    }

    #[test]
    fn joined_key_order_matches_field_by_field_order_for_escaped_values() {
        let groups = [
            [b"1".as_slice(), b"0".as_slice(), b"0", b"a", b"x"],
            [b"1", b"0", b"0", br"a\tb", b"x"],
            [b"1", b"0", b"0", b"ab", b"x"],
            [b"1", b"0", b"0", b"\\N", b"x"],
            [b"1", b"0", b"0", b"", b"x"],
            [b"10", b"0", b"0", b"a", b"x"],
        ];
        let joined = |fields: &[&[u8]; 5]| {
            let mut key = Vec::new();
            for (index, field) in fields.iter().enumerate() {
                if index > 0 {
                    key.push(b'\t');
                }
                key.extend_from_slice(field);
            }
            key
        };
        let mut field_order = groups.to_vec();
        field_order.sort_unstable();
        let mut joined_order = groups.iter().map(joined).collect::<Vec<_>>();
        joined_order.sort_unstable();
        let expected = field_order.iter().map(joined).collect::<Vec<_>>();
        assert_eq!(joined_order, expected);
    }
}
