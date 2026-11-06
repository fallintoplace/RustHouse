//! Keep typed cell validation and predicate compilation separate from the main scan loop.

use super::*;

pub(crate) fn execute_typed<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let projection = match &query.projection {
        Projection::All => metadata
            .columns
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>(),
        Projection::Columns(cols) => cols.clone(),
        Projection::Count => Vec::new(),
        Projection::CountColumn { column }
        | Projection::CountDistinct { column }
        | Projection::ApproxDistinct { column }
        | Projection::ApproxDistinctState { column }
        | Projection::GroupedCount { column } => {
            vec![column.clone()]
        }
        _ => {
            return Err(ExecutionError::Invalid(
                "typed SQL currently supports SELECT columns and COUNT aggregates".into(),
            ));
        }
    };
    let mut names = projection.clone();
    let mut include = |name: &str| {
        if !names.iter().any(|value| value == name) {
            names.push(name.to_owned());
        }
    };
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        include(predicate.column());
        if let Some(other) = predicate.other_column() {
            include(other);
        }
    }
    let sort = if matches!(query.projection, Projection::GroupedCount { .. }) {
        Vec::new()
    } else {
        query
            .order_terms()
            .map(|order| query.order_source_column(&order.column).to_owned())
            .collect::<Vec<_>>()
    };
    for name in &sort {
        include(name);
    }
    if names.is_empty() {
        return Err(ExecutionError::Invalid(
            "typed scan requires a column".into(),
        ));
    }
    if query.distinct && sort.iter().any(|name| !projection.contains(name)) {
        return Err(ExecutionError::Invalid(
            "DISTINCT ORDER BY column must appear in SELECT".into(),
        ));
    }
    let indices: HashMap<String, usize> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let types: HashMap<String, String> = metadata
        .columns
        .iter()
        .map(|col| (col.name.clone(), col.data_type.clone()))
        .collect();
    let projection_types = projection
        .iter()
        .map(|name| types.get(name).map(String::as_str).unwrap_or(""))
        .collect::<Vec<_>>();
    let first = compile(&query.predicates, &indices, &types)?;
    let mut conditions = vec![first];
    for branch in &query.or_predicates {
        conditions.push(compile(branch, &indices, &types)?);
    }
    let projection_indices = projection
        .iter()
        .map(|name| indices[name])
        .collect::<Vec<_>>();
    let sort_indices = sort
        .iter()
        .map(|name| *indices.get(name).unwrap())
        .collect::<Vec<_>>();
    let header = match &query.projection {
        Projection::Count => "count".to_string(),
        Projection::CountColumn { column } => format!("count({column})"),
        Projection::CountDistinct { column } => format!("countDistinct({column})"),
        Projection::ApproxDistinct { column } => format!("uniq({column})"),
        Projection::ApproxDistinctState { column } => format!("uniqState({column})"),
        Projection::GroupedCount { column } => {
            format!("{column}\t{}", query.alias.as_deref().unwrap_or("count"))
        }
        _ => projection.join("\t"),
    };
    if matches!(query.projection, Projection::GroupedCount { .. }) {
        writeln!(output, "{header}")?;
    } else {
        super::super::write_header(query, output, &header)?;
    }
    if query.limit == Some(0) {
        return Ok(());
    }
    // A constant NULL test needs only the validated schema and row count.
    if matches!(query.projection, Projection::Count)
        && !query.distinct
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
        && query.having.is_none()
    {
        if let Some(all_rows) = static_nonnullable_null_filter(&conditions, &names, &types) {
            if query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{}", if all_rows { metadata.rows } else { 0 })?;
            }
            return Ok(());
        }
    }
    // COUNT(nullable_column) only needs the per-row NULL bit. Decode its
    // small null-map stream instead of materializing possibly huge payloads.
    if let Projection::CountColumn { column } = &query.projection {
        if query.predicates.is_empty()
            && query.or_predicates.is_empty()
            && !query.distinct
            && query.order_by.is_none()
            && query.order_by_extra.is_empty()
            && types
                .get(column)
                .is_some_and(|kind| kind.starts_with("Nullable("))
        {
            let mut reader = TypedQueryReader::open_with_metadata(
                part,
                std::slice::from_ref(column),
                &metadata,
            )?;
            let mut count = 0u64;
            while let Some(nulls) = reader.next_null_batch()? {
                count = count
                    .checked_add(nulls.iter().filter(|&&byte| byte == 0).count() as u64)
                    .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
            }
            if query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{count}")?;
            }
            return Ok(());
        }
    }
    // COUNT(non-nullable column) is identical to COUNT(*). Read the
    // snapshot row count rather than decoding String/UUID/numeric streams.
    if let Projection::CountColumn { column } = &query.projection {
        if query.predicates.is_empty()
            && query.or_predicates.is_empty()
            && !query.distinct
            && query.order_by.is_none()
            && query.order_by_extra.is_empty()
            && metadata.columns.iter().any(|candidate| {
                candidate.name == *column && !candidate.data_type.starts_with("Nullable(")
            })
        {
            if query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{}", metadata.rows)?;
            }
            return Ok(());
        }
    }
    // COUNT(DISTINCT) has one scalar output. Parallel bitmaps avoid a
    // serial pass over many independent 32-bit MergeTree parts.
    if let Projection::CountDistinct { column } = &query.projection {
        if query.predicates.is_empty()
            && query.or_predicates.is_empty()
            && matches!(
                types.get(column).map(|kind| base_type(kind)),
                Some("UInt32" | "Int32")
            )
        {
            if let Some(count) = parallel_exact_u32_distinct(part, column)? {
                if query.offset.unwrap_or(0) == 0 {
                    writeln!(output, "{count}")?;
                }
                return Ok(());
            }
        }
    }
    // Parallel filtered COUNT can read independent immutable parts without
    // retaining result rows. Keep the scalar COUNT(*) case free of column indexing.
    if !query.distinct
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
        && query.having.is_none()
        && (!query.predicates.is_empty() || !query.or_predicates.is_empty())
        && matches!(
            query.projection,
            Projection::Count | Projection::CountColumn { .. }
        )
    {
        let column = match &query.projection {
            Projection::CountColumn { .. } => Some(projection_indices[0]),
            _ => None,
        };
        if let Some(total) = parallel_filtered_count(part, &names, &conditions, column)? {
            if query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{total}")?;
            }
            return Ok(());
        }
    }
    let mut scan = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let is_count = matches!(query.projection, Projection::Count);
    let is_count_column = matches!(query.projection, Projection::CountColumn { .. });
    let is_count_distinct = matches!(query.projection, Projection::CountDistinct { .. });
    let is_approx_distinct = matches!(
        query.projection,
        Projection::ApproxDistinct { .. } | Projection::ApproxDistinctState { .. }
    );
    let is_approx_state = matches!(query.projection, Projection::ApproxDistinctState { .. });
    let mut sketch = is_approx_distinct.then(HyperLogLog::default);
    let is_grouped = matches!(query.projection, Projection::GroupedCount { .. });
    let mut count = 0u64;
    let mut distinct = HashSet::<Vec<Key>>::new();
    let mut select_distinct_key_bytes = 0usize;
    // A one-column DISTINCT key does not need a heap-allocated Vec<Key>.
    let mut single_distinct = HashSet::<Key>::new();
    // Small integer domains need only 32 bytes (8-bit) or 8 KiB (16-bit)
    // to identify already emitted DISTINCT values, without hashing.
    let mut dense_single_distinct = if query.distinct && projection_indices.len() == 1 {
        match projection_types.first().copied() {
            Some("UInt8" | "Int8") => Some(vec![0u64; 4]),
            Some("UInt16" | "Int16") => Some(vec![0u64; 1024]),
            _ => None,
        }
    } else {
        None
    };
    let mut distinct_probe = Vec::<Key>::new();
    // COUNT(DISTINCT column) has one key; avoid a Vec allocation per row.
    let mut count_distinct_keys = HashSet::<Key>::new();
    // A nullable 16-bit domain fits into 8 KiB, independent of scan length.
    let mut count_distinct_sixteen = if is_count_distinct
        && matches!(
            projection_types.first().copied(),
            Some(
                "UInt8"
                    | "Int8"
                    | "UInt16"
                    | "Int16"
                    | "Nullable(UInt8)"
                    | "Nullable(Int8)"
                    | "Nullable(UInt16)"
                    | "Nullable(Int16)"
            )
        ) {
        Some(vec![
            0u64;
            if matches!(
                projection_types.first().copied(),
                Some("UInt8" | "Int8" | "Nullable(UInt8)" | "Nullable(Int8)")
            ) {
                4
            } else {
                1024
            }
        ])
    } else {
        None
    };
    // A dense, exact 32-bit bitmap handles high-cardinality nullable IDs.
    let mut count_distinct_thirty_two = if is_count_distinct
        && matches!(
            projection_types.first().copied(),
            Some("UInt32" | "Int32" | "Nullable(UInt32)" | "Nullable(Int32)")
        ) {
        Some(BitmapDistinct32::new())
    } else {
        None
    };
    // Borrow decoded strings on duplicate probes. Own only new distinct values.
    let mut count_distinct_text = HashSet::<Vec<u8>>::new();
    let mut count_distinct_text_bytes = 0usize;
    // UUIDs have a fixed-size identity. Hash their bytes rather than allocating
    // a formatted String for each scanned row.
    let mut count_distinct_uuids = HashSet::<[u8; 16]>::new();
    let mut group_indices = HashMap::<Key, usize>::new();
    // For narrow integer dimensions a direct bucket array removes one hash
    // lookup per matching fact row. Nullable keys still use the sparse path.
    let mut dense_group_indices = if is_grouped {
        match projection_types.first().copied() {
            Some("UInt8" | "Int8" | "Nullable(UInt8)" | "Nullable(Int8)") => {
                Some(vec![usize::MAX; 256])
            }
            Some("UInt16" | "Int16" | "Nullable(UInt16)" | "Nullable(Int16)") => {
                Some(vec![usize::MAX; 65_536])
            }
            _ => None,
        }
    } else {
        None
    };
    // String group probes can borrow the decoded input instead of cloning it.
    let mut text_group_indices = HashMap::<Vec<u8>, usize>::new();
    let mut grouped_text_bytes = 0usize;
    let mut uuid_group_indices = HashMap::<[u8; 16], usize>::new();
    let mut grouped_rows = Vec::<(Cell, u64)>::new();
    // Typed ORDER BY spills sorted runs when the bounded top-k heap cannot be used.
    let mut top = BinaryHeap::<TopRow>::new();
    let mut ordinal = 0usize;
    let mut emitted = 0usize;
    let mut skipped = 0usize;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(usize::MAX);
    // ORDER BY supports at most eight keys, allowing a compact direction mask.
    let mut descending_mask = 0u8;
    let mut nulls_first_mask = 0u8;
    let mut nulls_last_mask = 0u8;
    for (index, order) in query.order_terms().enumerate() {
        let bit = 1u8 << index;
        if order.descending {
            descending_mask |= bit;
        }
        match order.nulls_first {
            Some(true) => nulls_first_mask |= bit,
            Some(false) => nulls_last_mask |= bit,
            None => {}
        }
    }
    let mut external_sort =
        TypedExternalSort::new(descending_mask, nulls_first_mask, nulls_last_mask);
    let top_capacity = if sort_indices.is_empty() {
        None
    } else {
        query
            .limit
            .and_then(|count| count.checked_add(offset))
            .filter(|count| *count <= MAX_BUFFERED_ROWS)
    };
    // Allocate only the bounded set of winning rows. Avoid repeated heap
    // growth for typed ORDER BY while leaving unlimited scans lazy.
    if let Some(capacity) = top_capacity {
        top.reserve(capacity.min(1024));
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    if no_filters
        && !is_count
        && !is_count_column
        && !is_count_distinct
        && !is_approx_distinct
        && !is_grouped
        && !query.distinct
        && sort_indices.is_empty()
    {
        skipped = scan.skip_prefix_rows(offset)?;
    }
    while let Some(batch) = scan.next_batch()? {
        if no_filters && (is_count || is_count_column) {
            let matches = if is_count {
                batch[0].len()
            } else {
                match &batch[projection_indices[0]] {
                    TypedColumn::NullableNumeric { nulls, .. }
                    | TypedColumn::NullableText { nulls, .. } => {
                        nulls.iter().filter(|&&null| null == 0).count()
                    }
                    _ => batch[0].len(),
                }
            };
            count = count
                .checked_add(matches as u64)
                .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
            continue;
        }
        for index in 0..batch[0].len() {
            if !no_filters && !passes(&batch, index, &conditions) {
                continue;
            }
            if is_count || is_count_column {
                if is_count || !is_null_at(&batch[projection_indices[0]], index) {
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
                }
                continue;
            }
            if is_approx_distinct {
                sketch
                    .as_mut()
                    .expect("approximate DISTINCT sketch")
                    .observe(&batch[projection_indices[0]], index)?;
                continue;
            }
            if is_grouped {
                if let Some(slots) = dense_group_indices.as_mut() {
                    if let Some(bucket) = dense_group_bucket(&batch[projection_indices[0]], index) {
                        let group = &mut slots[bucket];
                        if *group != usize::MAX {
                            let count = &mut grouped_rows[*group].1;
                            *count = count.checked_add(1).ok_or_else(|| {
                                ExecutionError::Invalid("group count overflow".into())
                            })?;
                        } else {
                            let next = grouped_rows.len();
                            if next >= MAX_BUFFERED_ROWS {
                                return Err(ExecutionError::Invalid(
                                    "GROUP BY supports at most 200000 groups".into(),
                                ));
                            }
                            *group = next;
                            grouped_rows.push((cell(&batch[projection_indices[0]], index), 1));
                        }
                        continue;
                    }
                }
                let text_key = match &batch[projection_indices[0]] {
                    TypedColumn::Text(strings) => strings.get(index),
                    TypedColumn::NullableText { values, nulls } if nulls[index] == 0 => {
                        values.get(index)
                    }
                    _ => None,
                };
                if let Some(key) = text_key {
                    if let Some(&group) = text_group_indices.get(key) {
                        let count = &mut grouped_rows[group].1;
                        *count = count.checked_add(1).ok_or_else(|| {
                            ExecutionError::Invalid("group count overflow".into())
                        })?;
                    } else {
                        if grouped_rows.len() >= MAX_BUFFERED_ROWS {
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        let group = grouped_rows.len();
                        charge_grouped_text(&mut grouped_text_bytes, key.len())?;
                        text_group_indices.insert(key.to_vec(), group);
                        grouped_rows.push((Cell::Text(key.to_vec()), 1));
                    }
                    continue;
                }
                if let TypedColumn::Uuid(values) = &batch[projection_indices[0]] {
                    let key = *values[index].as_bytes();
                    if let Some(&group) = uuid_group_indices.get(&key) {
                        let count = &mut grouped_rows[group].1;
                        *count = count.checked_add(1).ok_or_else(|| {
                            ExecutionError::Invalid("group count overflow".into())
                        })?;
                    } else {
                        if grouped_rows.len() >= MAX_BUFFERED_ROWS {
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        let group = grouped_rows.len();
                        uuid_group_indices.insert(key, group);
                        grouped_rows.push((cell(&batch[projection_indices[0]], index), 1));
                    }
                    continue;
                }
                let value = cell(&batch[projection_indices[0]], index);
                match group_indices.entry(value.key()) {
                    Entry::Occupied(entry) => {
                        let count = &mut grouped_rows[*entry.get()].1;
                        *count = count.checked_add(1).ok_or_else(|| {
                            ExecutionError::Invalid("group count overflow".into())
                        })?;
                    }
                    Entry::Vacant(entry) => {
                        if grouped_rows.len() >= MAX_BUFFERED_ROWS {
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        entry.insert(grouped_rows.len());
                        grouped_rows.push((value, 1));
                    }
                }
                continue;
            }
            // In an ordinary unsorted scan, OFFSET can discard the row before
            // cloning String or UUID projection values.
            if !is_count_distinct && !query.distinct && sort_indices.is_empty() && skipped < offset
            {
                skipped += 1;
                continue;
            }
            // For non-DISTINCT top-k scans, sort keys determine whether a row
            // can enter the heap before we clone projected text or UUID values.
            let mut candidate_sort_keys = None;
            if !query.distinct
                && !is_count_distinct
                && !sort_indices.is_empty()
                && top_capacity.is_some_and(|capacity| capacity > 0 && top.len() >= capacity)
            {
                let keys = sort_indices
                    .iter()
                    .map(|&sort_index| cell(&batch[sort_index], index))
                    .collect::<Vec<_>>();
                if top.peek().is_some_and(|worst| {
                    compare_sort_keys(
                        &keys,
                        &worst.row.sort_keys,
                        descending_mask,
                        nulls_first_mask,
                        nulls_last_mask,
                    ) != Ordering::Less
                }) {
                    continue;
                }
                candidate_sort_keys = Some(keys);
            }
            if is_count_distinct {
                if let Some(bitmap) = count_distinct_thirty_two.as_mut() {
                    let column = &batch[projection_indices[0]];
                    if is_null_at(column, index) {
                        continue;
                    }
                    let value = match column {
                        TypedColumn::Numeric(NumericColumn::UInt32(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::UInt32(values),
                            ..
                        } => values[index],
                        TypedColumn::Numeric(NumericColumn::Int32(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::Int32(values),
                            ..
                        } => values[index] as u32,
                        _ => {
                            return Err(ExecutionError::Invalid(
                                "unexpected 32-bit COUNT(DISTINCT) column type".into(),
                            ));
                        }
                    };
                    bitmap.insert(value)?;
                    continue;
                }
                if let Some(bitmap) = count_distinct_sixteen.as_mut() {
                    let column = &batch[projection_indices[0]];
                    if is_null_at(column, index) {
                        continue;
                    }
                    let key = match column {
                        TypedColumn::Numeric(NumericColumn::UInt16(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::UInt16(values),
                            ..
                        } => values[index],
                        TypedColumn::Numeric(NumericColumn::Int16(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::Int16(values),
                            ..
                        } => values[index] as u16,
                        TypedColumn::Numeric(NumericColumn::UInt8(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::UInt8(values),
                            ..
                        } => u16::from(values[index]),
                        TypedColumn::Numeric(NumericColumn::Int8(values))
                        | TypedColumn::NullableNumeric {
                            values: NumericColumn::Int8(values),
                            ..
                        } => u16::from(values[index] as u8),
                        _ => {
                            return Err(ExecutionError::Invalid(
                                "unexpected 8-bit or 16-bit DISTINCT column type".into(),
                            ));
                        }
                    };
                    let index = usize::from(key);
                    bitmap[index / 64] |= 1u64 << (index % 64);
                    continue;
                }
                match &batch[projection_indices[0]] {
                    TypedColumn::Text(strings) => {
                        let value = strings.get(index).ok_or_else(|| {
                            ExecutionError::Invalid("invalid String row index".into())
                        })?;
                        remember_distinct_text(
                            &mut count_distinct_text,
                            &mut count_distinct_text_bytes,
                            value,
                        )?;
                    }
                    TypedColumn::NullableText { values, nulls } => {
                        if nulls[index] == 0 {
                            let value = values.get(index).ok_or_else(|| {
                                ExecutionError::Invalid("invalid String row index".into())
                            })?;
                            remember_distinct_text(
                                &mut count_distinct_text,
                                &mut count_distinct_text_bytes,
                                value,
                            )?;
                        }
                    }
                    TypedColumn::Uuid(values) => {
                        let key = *values[index].as_bytes();
                        if count_distinct_uuids.len() >= MAX_BUFFERED_ROWS {
                            if !count_distinct_uuids.contains(&key) {
                                return Err(ExecutionError::Invalid(
                                    "too many distinct rows".into(),
                                ));
                            }
                        } else {
                            count_distinct_uuids.insert(key);
                        }
                    }
                    column => {
                        let value = cell(column, index);
                        if !matches!(&value, Cell::Null) {
                            let key = value.key();
                            if count_distinct_keys.len() >= MAX_BUFFERED_ROWS {
                                if !count_distinct_keys.contains(&key) {
                                    return Err(ExecutionError::Invalid(
                                        "too many distinct rows".into(),
                                    ));
                                }
                            } else {
                                count_distinct_keys.insert(key);
                            }
                        }
                    }
                }
                continue;
            }
            if query.distinct {
                if projection_indices.len() == 1 {
                    // Avoid a second heap container around a single typed key.
                    if let Some(bitmap) = dense_single_distinct.as_mut() {
                        let key = match &batch[projection_indices[0]] {
                            TypedColumn::Numeric(NumericColumn::UInt8(values)) => {
                                u16::from(values[index])
                            }
                            TypedColumn::Numeric(NumericColumn::Int8(values)) => {
                                u16::from(values[index] as u8)
                            }
                            TypedColumn::Numeric(NumericColumn::UInt16(values)) => values[index],
                            TypedColumn::Numeric(NumericColumn::Int16(values)) => {
                                values[index] as u16
                            }
                            _ => unreachable!("dense DISTINCT requires a fixed-width integer"),
                        };
                        let key = usize::from(key);
                        let bit = 1u64 << (key % 64);
                        let word = &mut bitmap[key / 64];
                        if *word & bit != 0 {
                            continue;
                        }
                        *word |= bit;
                    } else {
                        let key = key_at(&batch[projection_indices[0]], index);
                        if single_distinct.contains(&key) {
                            continue;
                        }
                        if single_distinct.len() >= MAX_BUFFERED_ROWS {
                            return Err(ExecutionError::Invalid("too many distinct rows".into()));
                        }
                        charge_select_distinct_key(
                            &mut select_distinct_key_bytes,
                            std::slice::from_ref(&key),
                        )?;
                        single_distinct.insert(key);
                    }
                } else {
                    // Probe with just the hash key. Duplicate rows should never
                    // materialize a second set of projected String/UUID cells.
                    distinct_probe.clear();
                    for &idx in &projection_indices {
                        distinct_probe.push(key_at(&batch[idx], index));
                    }
                    if distinct.contains(distinct_probe.as_slice()) {
                        continue;
                    }
                    if distinct.len() >= MAX_BUFFERED_ROWS {
                        return Err(ExecutionError::Invalid("too many distinct rows".into()));
                    }
                    charge_select_distinct_key(&mut select_distinct_key_bytes, &distinct_probe)?;
                    distinct.insert(std::mem::take(&mut distinct_probe));
                    distinct_probe = Vec::with_capacity(projection_indices.len());
                }
            }
            // A single unsorted projection can stream directly from the batch.
            // Skip per-row heap allocations for strings and typed values.
            if !query.distinct && sort_indices.is_empty() && projection_indices.len() == 1 {
                match &batch[projection_indices[0]] {
                    TypedColumn::Uuid(values) => writeln!(output, "{}", values[index])?,
                    TypedColumn::Text(values) => {
                        let bytes = values.get(index).ok_or_else(|| {
                            ExecutionError::Invalid("missing String projection value".into())
                        })?;
                        write_text_bytes(output, bytes)?;
                        output.write_all(b"\n")?;
                    }
                    TypedColumn::NullableText { values, nulls } => {
                        if nulls[index] == 0 {
                            let bytes = values.get(index).ok_or_else(|| {
                                ExecutionError::Invalid("missing nullable String projection".into())
                            })?;
                            write_text_bytes(output, bytes)?;
                            output.write_all(b"\n")?;
                        } else {
                            output.write_all(b"\\N\n")?;
                        }
                    }
                    TypedColumn::Numeric(values) => {
                        let value = Cell::Numeric(number_at(values, index));
                        write_typed_value(output, &value, projection_types[0])?;
                        output.write_all(b"\n")?;
                    }
                    TypedColumn::NullableNumeric { values, nulls } => {
                        if nulls[index] == 0 {
                            let value = Cell::Numeric(number_at(values, index));
                            write_typed_value(output, &value, projection_types[0])?;
                            output.write_all(b"\n")?;
                        } else {
                            output.write_all(b"\\N\n")?;
                        }
                    }
                }
                emitted += 1;
                if emitted == limit {
                    return Ok(());
                }
                continue;
            }
            let cells = projection_indices
                .iter()
                .map(|&idx| cell(&batch[idx], index))
                .collect::<Vec<_>>();
            if !sort_indices.is_empty() {
                let row = Row {
                    values: cells,
                    sort_keys: candidate_sort_keys.unwrap_or_else(|| {
                        sort_indices
                            .iter()
                            .map(|&sort_index| cell(&batch[sort_index], index))
                            .collect()
                    }),
                };
                if let Some(capacity) = top_capacity {
                    let next = TopRow {
                        row,
                        ordinal,
                        descending_mask,
                        nulls_first_mask,
                        nulls_last_mask,
                    };
                    ordinal = ordinal
                        .checked_add(1)
                        .ok_or_else(|| ExecutionError::Invalid("row ordinal overflow".into()))?;
                    if top.len() < capacity {
                        top.push(next);
                    } else if top.peek().is_some_and(|worst| next < *worst) {
                        *top.peek_mut().unwrap() = next;
                    }
                } else {
                    external_sort.push(row)?;
                }
                continue;
            }
            if skipped < offset {
                skipped += 1;
                continue;
            }
            write_row(output, &cells, &projection_types)?;
            emitted += 1;
            if emitted == limit {
                return Ok(());
            }
        }
    }
    if is_grouped {
        if let Some(having) = &query.having {
            grouped_rows.retain(|(_, count)| {
                super::super::predicates::number_matches(
                    Number::Unsigned(u128::from(*count)),
                    Number::Unsigned(u128::from(having.value)),
                    having.operator,
                )
            });
        }
        let mut bounded_order = None;
        if query.order_by.is_some() {
            // Compile the ORDER BY expression flags once per grouped query.
            // Comparators may execute many times for a high-cardinality GROUP BY.
            let sort_terms = query
                .order_terms()
                .map(|term| (term.column == "count", term.descending, term.nulls_first))
                .collect::<Vec<_>>();
            let compare = |(left_key, left_count): &(Cell, u64),
                           (right_key, right_count): &(Cell, u64)| {
                for &(sort_count, descending, nulls_first) in &sort_terms {
                    let comparison = if sort_count {
                        let comparison = left_count.cmp(right_count);
                        if descending {
                            comparison.reverse()
                        } else {
                            comparison
                        }
                    } else {
                        compare_order_cells(left_key, right_key, descending, nulls_first)
                    };
                    if comparison != Ordering::Equal {
                        return comparison;
                    }
                }
                cmp(left_key, right_key)
            };
            let end = query.limit.and_then(|page| offset.checked_add(page));
            if let Some(end) = end.filter(|&end| end < grouped_rows.len()) {
                let mut indices = (0..grouped_rows.len()).collect::<Vec<_>>();
                let compare_index = |left: &usize, right: &usize| {
                    compare(&grouped_rows[*left], &grouped_rows[*right])
                        .then_with(|| left.cmp(right))
                };
                indices.select_nth_unstable_by(end, &compare_index);
                indices.truncate(end);
                indices.sort_unstable_by(compare_index);
                bounded_order = Some(indices);
            } else {
                grouped_rows.sort_by(compare);
            }
        }
        let write_group = |key: &Cell, count: u64, output: &mut W| -> Result<(), ExecutionError> {
            write_typed_value(output, key, projection_types[0])?;
            writeln!(output, "\t{count}")?;
            Ok(())
        };
        if let Some(indices) = bounded_order {
            for index in indices.into_iter().skip(offset) {
                write_group(&grouped_rows[index].0, grouped_rows[index].1, output)?;
            }
        } else {
            for (key, count) in grouped_rows.iter().skip(offset).take(limit) {
                write_group(key, *count, output)?;
            }
        }
        return Ok(());
    }
    if is_count || is_count_column || is_count_distinct || is_approx_distinct {
        if offset == 0 {
            if is_approx_state {
                writeln!(
                    output,
                    "{}",
                    sketch.as_ref().expect("HLL state sketch").encode_state()
                )?;
                return Ok(());
            }
            let result = if is_approx_distinct {
                sketch
                    .as_ref()
                    .expect("approximate DISTINCT sketch")
                    .estimate()
            } else if is_count || is_count_column {
                count
            } else {
                (count_distinct_keys.len()
                    + count_distinct_text.len()
                    + count_distinct_uuids.len()
                    + count_distinct_thirty_two
                        .as_ref()
                        .map_or(0, |bitmap| bitmap.len() as usize)
                    + count_distinct_sixteen.as_ref().map_or(0, |bitmap| {
                        bitmap
                            .iter()
                            .map(|bits| bits.count_ones() as usize)
                            .sum::<usize>()
                    })) as u64
            };
            writeln!(output, "{result}")?;
        }
        return Ok(());
    }
    if query.order_by.is_some() {
        if top_capacity.is_some() {
            for entry in top.into_sorted_vec().into_iter().skip(offset).take(limit) {
                write_row(output, &entry.row.values, &projection_types)?;
            }
        } else {
            external_sort.write(output, &projection_types, offset, limit)?;
        }
    }
    Ok(())
}
