//! Group the three GROUP BY executors and their spill and parallel helpers in one module.

use super::*;

/// Retry rare high-cardinality unordered GROUP BY scans using an exact
/// external key sorter. Small groups keep the in-memory hash fast path.
/// The retry rescans an immutable snapshot rather than attempting to recover
/// partial in-memory counts or double-count the final batch.
#[allow(clippy::too_many_arguments)]
fn spill_unordered_groups<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    kind: &str,
    scan_columns: &[String],
    comparisons: &[Vec<CompiledPredicate>],
    metadata: &rusthouse_storage::PartMetadata,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let signed = matches!(kind, "Int32" | "Int64");
    let mut sorter = ExternalGroupCount64::new(signed, false);
    let mut reader = NumericQueryReader::open_with_metadata(part, scan_columns, metadata)?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            if !no_filters && !batch_matches(&batch, index, comparisons) {
                continue;
            }
            let key = match &batch[0] {
                NumericColumn::UInt32(values) => u64::from(values[index]),
                NumericColumn::Int32(values) => values[index] as i64 as u64,
                NumericColumn::UInt64(values) => values[index],
                NumericColumn::Int64(values) => values[index] as u64,
                _ => {
                    return Err(ExecutionError::Invalid(
                        "external grouped scan requires 32/64-bit integer key".into(),
                    ));
                }
            };
            sorter.insert(key)?;
        }
    }
    writeln!(
        output,
        "{column}\t{}",
        query.alias.as_deref().unwrap_or("count")
    )?;
    let mut skipped = 0usize;
    let mut emitted = 0usize;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(usize::MAX);
    sorter.finish_until(|key, count| {
        if let Some(having) = &query.having {
            if !number_matches(
                Number::Unsigned(u128::from(count)),
                Number::Unsigned(u128::from(having.value)),
                having.operator,
            ) {
                return Ok(true);
            }
        }
        if skipped >= offset && emitted < limit {
            if signed {
                writeln!(output, "{}\t{count}", key as i64)?;
            } else {
                writeln!(output, "{key}\t{count}")?;
            }
            emitted += 1;
        }
        skipped = skipped
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("GROUP BY offset overflow".into()))?;
        Ok(emitted < limit)
    })
}

/// Parallel hash GROUP BY over Int32/UInt32 dimensions. Worker group counts
/// share a global budget; excessive cardinality returns None so the serial
/// path can adapt to external spill instead of failing on worker memory.
fn parallel_integer_grouped_count(
    part: &Path,
    names: &[String],
    comparisons: &[Vec<CompiledPredicate>],
    no_filters: bool,
    kind: &str,
) -> Result<Option<GroupedCounts>, ExecutionError> {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;
    const MAX_PARTIAL_KEYS: usize = 100_000;
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = super::super::query_workers(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    let chunk_size = parts.len().div_ceil(workers);
    let retained = AtomicUsize::new(0);
    let chunks = std::thread::scope(|scope| -> Result<_, ExecutionError> {
        let handles = parts
            .chunks(chunk_size)
            .map(|paths| {
                let retained = &retained;
                std::thread::Builder::new()
                    .spawn_scoped(
                        scope,
                        move || -> Result<Option<Vec<(u64, u64)>>, ExecutionError> {
                            let mut keys = HashMap::<u64, usize>::new();
                            let mut counts = Vec::<(u64, u64)>::new();
                            for path in paths {
                                let mut reader =
                                    rusthouse_storage::NumericPartReader::open(path, names)?;
                                while let Some(batch) = reader.next_batch()? {
                                    let index = &batch[0];
                                    let mut visit = |key: u64| -> Result<bool, ExecutionError> {
                                        match keys.entry(key) {
                                            std::collections::hash_map::Entry::Occupied(entry) => {
                                                let count = &mut counts[*entry.get()].1;
                                                *count = count.checked_add(1).ok_or_else(|| {
                                                    ExecutionError::Invalid(
                                                        "parallel GROUP BY count overflow".into(),
                                                    )
                                                })?;
                                            }
                                            std::collections::hash_map::Entry::Vacant(entry) => {
                                                if !super::super::try_reserve_shared(
                                                    retained,
                                                    1,
                                                    MAX_PARTIAL_KEYS,
                                                ) {
                                                    return Ok(false);
                                                }
                                                entry.insert(counts.len());
                                                counts.push((key, 1));
                                            }
                                        }
                                        Ok(true)
                                    };
                                    for row in 0..index.len() {
                                        if !no_filters && !batch_matches(&batch, row, comparisons) {
                                            continue;
                                        }
                                        let key = match (index, kind) {
                                            (NumericColumn::UInt32(values), "UInt32") => {
                                                u64::from(values[row])
                                            }
                                            (NumericColumn::Int32(values), "Int32") => {
                                                values[row] as i64 as u64
                                            }
                                            (NumericColumn::UInt64(values), "UInt64") => values[row],
                                            (NumericColumn::Int64(values), "Int64") => {
                                                values[row] as u64
                                            }
                                            _ => {
                                                return Err(ExecutionError::Invalid(
                                                    "parallel integer grouping encountered unexpected column"
                                                        .into(),
                                                ));
                                            }
                                        };
                                        if !visit(key)? {
                                            return Ok(None);
                                        }
                                    }
                                }
                            }
                            Ok(Some(counts))
                        },
                    )
                    .map_err(|error| {
                        ExecutionError::Invalid(format!(
                            "cannot start hashed integer GROUP BY worker: {error}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(ExecutionError::Invalid(
                        "parallel grouped worker panicked".into(),
                    ))
                })
            })
            .collect::<Vec<_>>())
    })?;
    let mut keys = HashMap::<u64, usize>::new();
    let mut counts = Vec::<(u64, u64)>::new();
    for chunk in chunks {
        let Some(chunk) = chunk? else {
            return Ok(None);
        };
        for (key, count) in chunk {
            match keys.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    let total = &mut counts[*entry.get()].1;
                    *total = total.checked_add(count).ok_or_else(|| {
                        ExecutionError::Invalid("parallel GROUP BY count overflow".into())
                    })?;
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    if counts.len() >= MAX_PARTIAL_KEYS {
                        return Ok(None);
                    }
                    entry.insert(counts.len());
                    counts.push((key, count));
                }
            }
        }
    }
    let mut result = GroupedCounts::default();
    for (key, count) in counts {
        let value = if matches!(kind, "Int32" | "Int64") {
            Number::Signed(i128::from(key as i64))
        } else {
            Number::Unsigned(u128::from(key))
        };
        result.rows.push((value, count));
    }
    Ok(Some(result))
}

pub(crate) fn execute_grouped_part<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut scan_columns = vec![column.to_owned()];
    let comparisons = compile_part_where(query, &mut scan_columns, |column_name, literal| {
        let metadata_column = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == column_name)
            .ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown WHERE column: {column_name}"))
            })?;
        parse_numeric(&metadata_column.data_type, literal)
    })?;
    let mut reader = NumericQueryReader::open_with_metadata(part, &scan_columns, &metadata)?;
    let mut groups = GroupedCounts::default();
    if query.limit == Some(0) {
        return groups.write(query, column, output);
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    // High-cardinality 32/64-bit GROUP BY with ORDER BY key is exact even when
    // the distinct group set cannot fit in memory. Sorted spill runs avoid
    // the fixed 200k-group HashMap ceiling and preserve ORDER BY semantics.
    let kind = metadata
        .columns
        .iter()
        .find(|c| c.name == column)
        .map(|c| c.data_type.as_str());
    let ordering = query
        .order_by
        .as_ref()
        .filter(|order| order.column == column && query.order_by_extra.is_empty());
    if let (Some(kind @ ("UInt32" | "Int32" | "UInt64" | "Int64")), Some(order)) = (kind, ordering)
    {
        let signed = matches!(kind, "Int32" | "Int64");
        let mut sorter = ExternalGroupCount64::new(signed, order.descending);
        while let Some(batch) = reader.next_batch()? {
            for row in 0..batch[0].len() {
                if !no_filters && !batch_matches(&batch, row, &comparisons) {
                    continue;
                }
                let value = match &batch[0] {
                    NumericColumn::UInt32(values) => u64::from(values[row]),
                    NumericColumn::Int32(values) => values[row] as i64 as u64,
                    NumericColumn::UInt64(values) => values[row],
                    NumericColumn::Int64(values) => values[row] as u64,
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected external GROUP BY key type".into(),
                        ));
                    }
                };
                sorter.insert(value)?;
            }
        }
        writeln!(
            output,
            "{column}\t{}",
            query.alias.as_deref().unwrap_or("count")
        )?;
        let mut seen = 0usize;
        let mut emitted = 0usize;
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(usize::MAX);
        sorter.finish_until(|key, count| {
            if let Some(having) = &query.having {
                if !number_matches(
                    Number::Unsigned(u128::from(count)),
                    Number::Unsigned(u128::from(having.value)),
                    having.operator,
                ) {
                    return Ok(true);
                }
            }
            if seen >= offset && emitted < limit {
                if signed {
                    writeln!(output, "{}\t{count}", key as i64)?;
                } else {
                    writeln!(output, "{key}\t{count}")?;
                }
                emitted += 1;
            }
            seen = seen
                .checked_add(1)
                .ok_or_else(|| ExecutionError::Invalid("GROUP BY row count overflow".into()))?;
            Ok(emitted < limit)
        })?;
        return Ok(());
    }
    if no_filters {
        if let Some(kind) = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == column)
            .map(|candidate| candidate.data_type.as_str())
            .filter(|kind| matches!(*kind, "UInt8" | "Int8" | "UInt16" | "Int16"))
        {
            if let Some(groups) = parallel_dense_grouped_count(part, column, kind)? {
                return groups.write(query, column, output);
            }
        }
    }
    // A single-byte group key has only 256 possible values. Accumulate in a
    // dense array, retaining the first-seen key order for unordered queries.
    let narrow = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == column)
        .map(|candidate| candidate.data_type.as_str());
    if matches!(narrow, Some("UInt8" | "Int8")) {
        let mut counts = [0u64; 256];
        let mut visited = [false; 256];
        let mut insertion_order = Vec::new();
        while let Some(batch) = reader.next_batch()? {
            for row in 0..batch[0].len() {
                if !no_filters && !batch_matches(&batch, row, &comparisons) {
                    continue;
                }
                let byte = match &batch[0] {
                    NumericColumn::UInt8(values) => values[row],
                    NumericColumn::Int8(values) => values[row] as u8,
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected byte-width GROUP BY type".into(),
                        ));
                    }
                };
                if !visited[byte as usize] {
                    visited[byte as usize] = true;
                    insertion_order.push(byte);
                }
                counts[byte as usize] = counts[byte as usize]
                    .checked_add(1)
                    .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
            }
        }
        for byte in insertion_order {
            let value = if matches!(narrow, Some("Int8")) {
                Number::Signed(i128::from(byte as i8))
            } else {
                Number::Unsigned(u128::from(byte))
            };
            // Dense keys were already deduplicated by their slot.
            // The final writer consumes only rows, not the hash index.
            groups.rows.push((value, counts[byte as usize]));
        }
        return groups.write(query, column, output);
    }
    // 16-bit keys have at most 65,536 groups. A dense counter array avoids
    // one HashMap probe and NumberKey construction per input row.
    if matches!(narrow, Some("UInt16" | "Int16")) {
        let mut counts = vec![0u64; 1 << 16];
        let mut insertion_order = Vec::<u16>::new();
        while let Some(batch) = reader.next_batch()? {
            for row in 0..batch[0].len() {
                if !no_filters && !batch_matches(&batch, row, &comparisons) {
                    continue;
                }
                let key = match &batch[0] {
                    NumericColumn::UInt16(values) => values[row],
                    NumericColumn::Int16(values) => values[row] as u16,
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected 16-bit GROUP BY type".into(),
                        ));
                    }
                };
                let count = &mut counts[usize::from(key)];
                if *count == 0 {
                    insertion_order.push(key);
                }
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
            }
        }
        for key in insertion_order {
            let value = if matches!(narrow, Some("Int16")) {
                Number::Signed(i128::from(key as i16))
            } else {
                Number::Unsigned(u128::from(key))
            };
            groups.rows.push((value, counts[usize::from(key)]));
        }
        return groups.write(query, column, output);
    }
    if matches!(narrow, Some("UInt32" | "Int32" | "UInt64" | "Int64")) {
        if let Some(result) = parallel_integer_grouped_count(
            part,
            &scan_columns,
            &comparisons,
            no_filters,
            narrow.expect("integer group kind was validated"),
        )? {
            return result.write(query, column, output);
        }
    }
    // Keep 32-bit group hash keys compact instead of widening each input
    // number into a NumberKey. Retain the first-seen group order.
    if matches!(narrow, Some("UInt32" | "Int32")) {
        let mut indices = HashMap::<u32, usize>::new();
        while let Some(batch) = reader.next_batch()? {
            for row in 0..batch[0].len() {
                if !no_filters && !batch_matches(&batch, row, &comparisons) {
                    continue;
                }
                let key = match &batch[0] {
                    NumericColumn::UInt32(values) => values[row],
                    NumericColumn::Int32(values) => values[row] as u32,
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected 32-bit GROUP BY type".into(),
                        ));
                    }
                };
                match indices.entry(key) {
                    Entry::Occupied(entry) => {
                        let count = &mut groups.rows[*entry.get()].1;
                        *count = count.checked_add(1).ok_or_else(|| {
                            ExecutionError::Invalid("group count overflow".into())
                        })?;
                    }
                    Entry::Vacant(entry) => {
                        if groups.rows.len() >= MAX_GROUPS {
                            // GROUP BY without ORDER BY has no stable row-order
                            // contract. Spill and globally merge exact counts
                            // only when unique keys exceed the hash ceiling.
                            if query.order_by.is_none() {
                                return spill_unordered_groups(
                                    query,
                                    part,
                                    column,
                                    narrow.expect("validated 32/64-bit key"),
                                    &scan_columns,
                                    &comparisons,
                                    &metadata,
                                    output,
                                );
                            }
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        let value = if matches!(narrow, Some("Int32")) {
                            Number::Signed(i128::from(key as i32))
                        } else {
                            Number::Unsigned(u128::from(key))
                        };
                        entry.insert(groups.rows.len());
                        groups.rows.push((value, 1));
                    }
                }
            }
        }
        return groups.write(query, column, output);
    }
    // 64-bit group keys do not need a 128-bit NumberKey on every row.
    // Keep an ordered list of first-seen groups for stable unordered results.
    if matches!(narrow, Some("UInt64" | "Int64")) {
        let mut indices = HashMap::<u64, usize>::new();
        while let Some(batch) = reader.next_batch()? {
            for row in 0..batch[0].len() {
                if !no_filters && !batch_matches(&batch, row, &comparisons) {
                    continue;
                }
                let key = match &batch[0] {
                    NumericColumn::UInt64(values) => values[row],
                    NumericColumn::Int64(values) => values[row] as u64,
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected 64-bit GROUP BY type".into(),
                        ));
                    }
                };
                match indices.entry(key) {
                    Entry::Occupied(entry) => {
                        let count = &mut groups.rows[*entry.get()].1;
                        *count = count.checked_add(1).ok_or_else(|| {
                            ExecutionError::Invalid("group count overflow".into())
                        })?;
                    }
                    Entry::Vacant(entry) => {
                        if groups.rows.len() >= MAX_GROUPS {
                            // GROUP BY without ORDER BY has no stable row-order
                            // contract. Spill and globally merge exact counts
                            // only when unique keys exceed the hash ceiling.
                            if query.order_by.is_none() {
                                return spill_unordered_groups(
                                    query,
                                    part,
                                    column,
                                    narrow.expect("validated 32/64-bit key"),
                                    &scan_columns,
                                    &comparisons,
                                    &metadata,
                                    output,
                                );
                            }
                            return Err(ExecutionError::Invalid(
                                "GROUP BY supports at most 200000 groups".into(),
                            ));
                        }
                        let value = if matches!(narrow, Some("Int64")) {
                            Number::Signed(i128::from(key as i64))
                        } else {
                            Number::Unsigned(u128::from(key))
                        };
                        entry.insert(groups.rows.len());
                        groups.rows.push((value, 1));
                    }
                }
            }
        }
        return groups.write(query, column, output);
    }
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if no_filters || batch_matches(&batch, row, &comparisons) {
                groups.push(number_at(&batch[0], row))?;
            }
        }
    }
    groups.write(query, column, output)
}

pub(crate) fn execute_grouped_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if column != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    let comparisons = compile_numbers_predicates(query)?;
    let mut groups = GroupedCounts::default();
    if query.limit == Some(0) {
        return groups.write(query, column, output);
    }
    // Unique generated keys need neither a hash table nor retained rows
    // when there is no grouping filter or sort. Preserve the group cap.
    if query.limit.is_none()
        && query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && query.having.is_none()
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
        && step != 0
    {
        if rows > MAX_GROUPS as u64 {
            return Err(ExecutionError::Invalid(
                "GROUP BY supports at most 200000 groups".into(),
            ));
        }
        writeln!(
            output,
            "{column}\t{}",
            query.alias.as_deref().unwrap_or("count")
        )?;
        for index in query.offset.unwrap_or(0) as u64..rows {
            let number = u128::from(start) + u128::from(index) * u128::from(step);
            writeln!(output, "{number}\t1")?;
        }
        return Ok(());
    }
    // Key order already follows (or reverses) the generated arithmetic
    // progression. No per-key hash state or full result sort is needed.
    if query.limit.is_none()
        && query.having.is_none()
        && query.order_by_extra.is_empty()
        && step != 0
    {
        if let Some(order) = query
            .order_by
            .as_ref()
            .filter(|order| order.column == "number")
        {
            let bounds = if query.predicates.is_empty() && query.or_predicates.is_empty() {
                Some((0, rows))
            } else {
                monotonic_range_indices(&comparisons, start, rows, step)
            };
            if let Some((lower, upper)) = bounds {
                let selected = upper - lower;
                if selected > MAX_GROUPS as u64 {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 200000 groups".into(),
                    ));
                }
                writeln!(
                    output,
                    "{column}\t{}",
                    query.alias.as_deref().unwrap_or("count")
                )?;
                let begin = (query.offset.unwrap_or(0) as u64).min(selected);
                for position in begin..selected {
                    let index = if order.descending {
                        upper - 1 - position
                    } else {
                        lower + position
                    };
                    let value = u128::from(start) + u128::from(index) * u128::from(step);
                    writeln!(output, "{value}\t1")?;
                }
                return Ok(());
            }
        }
    }
    // Group keys generated with a positive step are already sorted.
    // ORDER BY number can address any bounded ASC/DESC page without
    // constructing or sorting a group map, even at billion-row scale.
    if query.having.is_none() && query.order_by_extra.is_empty() {
        if let (Some(order), Some(limit)) = (query.order_by.as_ref(), query.limit) {
            if order.column == "number" {
                let range = if query.predicates.is_empty() && query.or_predicates.is_empty() {
                    Some((0, rows))
                } else {
                    monotonic_range_indices(&comparisons, start, rows, step)
                };
                if let Some((lower, upper)) = range {
                    let available = upper - lower;
                    let skip = (query.offset.unwrap_or(0) as u64).min(available);
                    let length = (limit as u64).min(available - skip);
                    writeln!(
                        output,
                        "{column}\t{}",
                        query.alias.as_deref().unwrap_or("count")
                    )?;
                    for position in skip..skip + length {
                        let index = if order.descending {
                            upper - 1 - position
                        } else {
                            lower + position
                        };
                        let number = u128::from(start) + u128::from(index) * u128::from(step);
                        writeln!(output, "{number}\t1")?;
                    }
                    return Ok(());
                }
            }
        }
    }
    // GROUP BY number contains exactly one row per generated key. If the
    // client requests a bounded unordered page, jump directly to its keys.
    if query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && query.having.is_none()
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
    {
        if let Some(limit) = query.limit {
            let begin = (query.offset.unwrap_or(0) as u64).min(rows);
            let length = (limit as u64).min(rows - begin);
            writeln!(
                output,
                "{column}\t{}",
                query.alias.as_deref().unwrap_or("count")
            )?;
            for index in begin..begin + length {
                let number = u128::from(start) + u128::from(index) * u128::from(step);
                writeln!(output, "{number}\t1")?;
            }
            return Ok(());
        }
    }
    if query.limit.is_none()
        && (!query.predicates.is_empty() || !query.or_predicates.is_empty())
        && query.having.is_none()
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
        && step != 0
    {
        if let Some((lower, upper)) = monotonic_range_indices(&comparisons, start, rows, step) {
            if upper - lower > MAX_GROUPS as u64 {
                return Err(ExecutionError::Invalid(
                    "GROUP BY supports at most 200000 groups".into(),
                ));
            }
            writeln!(
                output,
                "{column}\t{}",
                query.alias.as_deref().unwrap_or("count")
            )?;
            let begin = lower
                .saturating_add(query.offset.unwrap_or(0) as u64)
                .min(upper);
            for index in begin..upper {
                let number = u128::from(start) + u128::from(index) * u128::from(step);
                writeln!(output, "{number}\t1")?;
            }
            return Ok(());
        }
    }
    // For a filtered monotone range, unique groups can be addressed by
    // their source indices. Skip directly to OFFSET and emit LIMIT groups.
    if (!query.predicates.is_empty() || !query.or_predicates.is_empty())
        && query.having.is_none()
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
    {
        if let Some(limit) = query.limit {
            if let Some((lower, upper)) = monotonic_range_indices(&comparisons, start, rows, step) {
                let available = upper - lower;
                let skip = (query.offset.unwrap_or(0) as u64).min(available);
                let length = (limit as u64).min(available - skip);
                writeln!(
                    output,
                    "{column}\t{}",
                    query.alias.as_deref().unwrap_or("count")
                )?;
                for index in lower + skip..lower + skip + length {
                    let number = u128::from(start) + u128::from(index) * u128::from(step);
                    writeln!(output, "{number}\t1")?;
                }
                return Ok(());
            }
        }
    }
    for index in 0..rows {
        let number = start + index * step;
        let value = Number::Unsigned(u128::from(number));
        if number_matches_all(value, &comparisons) {
            groups.push(value)?;
        }
    }
    groups.write(query, column, output)
}

pub(crate) fn execute_grouped_aggregate_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::GroupedAggregate {
        column,
        function,
        value_column,
    } = &query.projection
    else {
        unreachable!("grouped aggregate source requires a grouped projection")
    };
    if column != "number" || value_column != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    let comparisons = compile_numbers_predicates(query)?;
    writeln!(output, "{column}\t{}({value_column})", function.name())?;
    if query.limit == Some(0) {
        return Ok(());
    }
    // Generated GROUP BY keys are unique and monotone, so paging by the
    // grouping key needs no hash table or full sort.
    if query.having.is_none() && query.order_by_extra.is_empty() && step != 0 {
        if let (Some(order), Some(limit)) = (query.order_by.as_ref(), query.limit) {
            if order.column == "number" {
                let range = if query.predicates.is_empty() && query.or_predicates.is_empty() {
                    Some((0, rows))
                } else {
                    monotonic_range_indices(&comparisons, start, rows, step)
                };
                if let Some((lower, upper)) = range {
                    let available = upper - lower;
                    let skip = (query.offset.unwrap_or(0) as u64).min(available);
                    let length = (limit as u64).min(available - skip);
                    for position in skip..skip + length {
                        let index = if order.descending {
                            upper - 1 - position
                        } else {
                            lower + position
                        };
                        let number = u128::from(start) + u128::from(index) * u128::from(step);
                        let mut state = AggregateState::new(*function);
                        state.update(Number::Unsigned(number))?;
                        writeln!(output, "{number}\t{}", state.render())?;
                    }
                    return Ok(());
                }
            }
        }
    }
    // Unique generated keys do not need per-key hash entries when the
    // result is emitted in source order. Reject oversized groups up front.
    if query.limit.is_none()
        && query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && query.having.is_none()
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
        && step != 0
    {
        if rows > MAX_GROUPS as u64 {
            return Err(ExecutionError::Invalid(
                "GROUP BY supports at most 100000 groups".into(),
            ));
        }
        for index in query.offset.unwrap_or(0) as u64..rows {
            let number = u128::from(start) + u128::from(index) * u128::from(step);
            let mut state = AggregateState::new(*function);
            state.update(Number::Unsigned(number))?;
            writeln!(output, "{number}\t{}", state.render())?;
        }
        return Ok(());
    }
    if query.limit.is_none()
        && query.having.is_none()
        && query.order_by_extra.is_empty()
        && step != 0
    {
        if let Some(order) = query
            .order_by
            .as_ref()
            .filter(|order| order.column == "number")
        {
            let bounds = if query.predicates.is_empty() && query.or_predicates.is_empty() {
                Some((0, rows))
            } else {
                monotonic_range_indices(&comparisons, start, rows, step)
            };
            if let Some((lower, upper)) = bounds {
                let selected = upper - lower;
                if selected > MAX_GROUPS as u64 {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 100000 groups".into(),
                    ));
                }
                let begin = (query.offset.unwrap_or(0) as u64).min(selected);
                for position in begin..selected {
                    let index = if order.descending {
                        upper - 1 - position
                    } else {
                        lower + position
                    };
                    let value = u128::from(start) + u128::from(index) * u128::from(step);
                    let mut state = AggregateState::new(*function);
                    state.update(Number::Unsigned(value))?;
                    writeln!(output, "{value}\t{}", state.render())?;
                }
                return Ok(());
            }
        }
    }
    // A generated number is a unique group key. Each unfiltered group
    // contains one row, so a bounded page needs only its requested keys.
    if query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && query.having.is_none()
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
    {
        if let Some(limit) = query.limit {
            let begin = (query.offset.unwrap_or(0) as u64).min(rows);
            let length = (limit as u64).min(rows - begin);
            for index in begin..begin + length {
                let number = u128::from(start) + u128::from(index) * u128::from(step);
                let mut state = AggregateState::new(*function);
                state.update(Number::Unsigned(number))?;
                writeln!(output, "{number}\t{}", state.render())?;
            }
            return Ok(());
        }
    }
    if query.limit.is_none()
        && (!query.predicates.is_empty() || !query.or_predicates.is_empty())
        && query.having.is_none()
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
        && step != 0
    {
        if let Some((lower, upper)) = monotonic_range_indices(&comparisons, start, rows, step) {
            if upper - lower > MAX_GROUPS as u64 {
                return Err(ExecutionError::Invalid(
                    "GROUP BY supports at most 100000 groups".into(),
                ));
            }
            let begin = lower
                .saturating_add(query.offset.unwrap_or(0) as u64)
                .min(upper);
            for index in begin..upper {
                let number = u128::from(start) + u128::from(index) * u128::from(step);
                let mut state = AggregateState::new(*function);
                state.update(Number::Unsigned(number))?;
                writeln!(output, "{number}\t{}", state.render())?;
            }
            return Ok(());
        }
    }
    // Each generated key occurs exactly once. For simple filtered ranges,
    // evaluate one-row aggregate states only for the requested page.
    if (!query.predicates.is_empty() || !query.or_predicates.is_empty())
        && query.having.is_none()
        && query.order_by.is_none()
        && query.order_by_extra.is_empty()
    {
        if let Some(limit) = query.limit {
            if let Some((lower, upper)) = monotonic_range_indices(&comparisons, start, rows, step) {
                let available = upper - lower;
                let skip = (query.offset.unwrap_or(0) as u64).min(available);
                let length = (limit as u64).min(available - skip);
                for index in lower + skip..lower + skip + length {
                    let number = u128::from(start) + u128::from(index) * u128::from(step);
                    let mut state = AggregateState::new(*function);
                    state.update(Number::Unsigned(number))?;
                    writeln!(output, "{number}\t{}", state.render())?;
                }
                return Ok(());
            }
        }
    }
    let mut by_key = HashMap::<NumberKey, usize>::new();
    let mut groups = Vec::<(Number, AggregateState)>::new();
    for index in 0..rows {
        let number = start + index * step;
        let value = Number::Unsigned(u128::from(number));
        if !number_matches_all(value, &comparisons) {
            continue;
        }
        match by_key.entry(NumberKey::from(value)) {
            Entry::Occupied(entry) => groups[*entry.get()].1.update(value)?,
            Entry::Vacant(entry) => {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 100000 groups".into(),
                    ));
                }
                entry.insert(groups.len());
                let mut state = AggregateState::new(*function);
                state.update(value)?;
                groups.push((value, state));
            }
        }
    }
    if let Some(having) = &query.having {
        groups.retain(|(_, state)| state.passes_having(having));
    }
    if query.order_by.is_some() {
        groups.sort_by(|(left_key, left_state), (right_key, right_state)| {
            for term in query.order_terms() {
                let comparison = if term.column == "#aggregate" {
                    typed::compare_order_cells(
                        &left_state.sort_cell(),
                        &right_state.sort_cell(),
                        term.descending,
                        term.nulls_first,
                    )
                } else {
                    let ordering = compare_numbers(*left_key, *right_key);
                    if term.descending {
                        ordering.reverse()
                    } else {
                        ordering
                    }
                };
                if comparison != Ordering::Equal {
                    return comparison;
                }
            }
            Ordering::Equal
        });
    }
    for (key, state) in groups
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        writeln!(output, "{key}\t{}", state.render())?;
    }
    Ok(())
}

