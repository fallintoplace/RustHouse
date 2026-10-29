use super::*;

struct DenseGroupChunk {
    counts: Vec<u64>,
    order: Vec<u16>,
}

/// Parallelize low-cardinality GROUP BY over many immutable parts. Each worker
/// keeps fixed-size counters rather than one hash entry per input row.
/// Merge chunks in part order to retain stable first-seen output order.
fn parallel_dense_grouped_count(
    part: &Path,
    column: &str,
    kind: &str,
) -> Result<Option<GroupedCounts>, ExecutionError> {
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    let slots = if matches!(kind, "Int8" | "UInt8") {
        256
    } else {
        65_536
    };
    let names = [column.to_owned()];
    let chunk_size = parts.len().div_ceil(workers);
    let chunks = std::thread::scope(|scope| {
        let names = &names;
        let handles = parts
            .chunks(chunk_size)
            .map(|paths| {
                scope.spawn(move || -> Result<DenseGroupChunk, ExecutionError> {
                    let mut counts = vec![0u64; slots];
                    let mut order = Vec::new();
                    for path in paths {
                        let mut reader = rusthouse_storage::NumericPartReader::open(path, names)?;
                        while let Some(batch) = reader.next_batch()? {
                            match &batch[0] {
                                NumericColumn::UInt8(values) => {
                                    for &value in values {
                                        let slot = usize::from(value);
                                        if counts[slot] == 0 {
                                            order.push(value.into());
                                        }
                                        counts[slot] = counts[slot].checked_add(1)
                                            .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
                                    }
                                }
                                NumericColumn::Int8(values) => {
                                    for &value in values {
                                        let slot = usize::from(value as u8);
                                        if counts[slot] == 0 {
                                            order.push(slot as u16);
                                        }
                                        counts[slot] = counts[slot].checked_add(1)
                                            .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
                                    }
                                }
                                NumericColumn::UInt16(values) => {
                                    for &value in values {
                                        let slot = usize::from(value);
                                        if counts[slot] == 0 {
                                            order.push(value);
                                        }
                                        counts[slot] = counts[slot].checked_add(1)
                                            .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
                                    }
                                }
                                NumericColumn::Int16(values) => {
                                    for &value in values {
                                        let slot = usize::from(value as u16);
                                        if counts[slot] == 0 {
                                            order.push(slot as u16);
                                        }
                                        counts[slot] = counts[slot].checked_add(1)
                                            .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
                                    }
                                }
                                _ => return Err(ExecutionError::Invalid("unexpected grouped key type".into())),
                            }
                        }
                    }
                    Ok(DenseGroupChunk { counts, order })
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_else(|_| {
                Err(ExecutionError::Invalid("grouped scan worker panicked".into()))
            }))
            .collect::<Vec<_>>()
    });
    let mut counts = vec![0u64; slots];
    let mut order = Vec::new();
    for chunk in chunks {
        let chunk = chunk?;
        for key in chunk.order {
            let slot = usize::from(key);
            if counts[slot] == 0 {
                order.push(key);
            }
            counts[slot] = counts[slot].checked_add(chunk.counts[slot])
                .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
        }
    }
    let mut groups = GroupedCounts::default();
    for key in order {
        let value = match kind {
            "Int8" => Number::Signed(i128::from(key as u8 as i8)),
            "Int16" => Number::Signed(i128::from(key as i16)),
            _ => Number::Unsigned(u128::from(key)),
        };
        groups.rows.push((value, counts[usize::from(key)]));
    }
    Ok(Some(groups))
}


/// Retry rare high-cardinality unordered GROUP BY scans using an exact
/// external key sorter. Small groups keep the in-memory hash fast path.
/// The retry rescans an immutable snapshot rather than attempting to recover
/// partial in-memory counts or double-count the final batch.
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
    let mut reader = NumericQueryReader::open_with_metadata(
        part, scan_columns, metadata,
    )?;
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
                _ => return Err(ExecutionError::Invalid(
                    "external grouped scan requires 32/64-bit integer key".into(),
                )),
            };
            sorter.insert(key)?;
        }
    }
    writeln!(output, "{column}\t{}",
        query.alias.as_deref().unwrap_or("count"))?;
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
        skipped = skipped.checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("GROUP BY offset overflow".into()))?;
        Ok(emitted < limit)
    })
}

/// Parallel hash GROUP BY over Int32/UInt32 dimensions. Worker group counts
/// share a global budget; excessive cardinality returns None so the serial
/// path can adapt to external spill instead of failing on worker memory.
fn parallel_integer_grouped_count(
    part: &Path,
    column: &str,
    kind: &str,
) -> Result<Option<GroupedCounts>, ExecutionError> {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    const MAX_PARTIAL_KEYS: usize = 100_000;
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get()).min(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    let names = [column.to_owned()];
    let chunk_size = parts.len().div_ceil(workers);
    let retained = AtomicUsize::new(0);
    let chunks = std::thread::scope(|scope| {
        let handles = parts.chunks(chunk_size).map(|paths| {
            let names = &names;
            let retained = &retained;
            scope.spawn(move || -> Result<Option<Vec<(u64, u64)>>, ExecutionError> {
                let mut keys = HashMap::<u64, usize>::new();
                let mut counts = Vec::<(u64, u64)>::new();
                for path in paths {
                    let mut reader = rusthouse_storage::NumericPartReader::open(path, names)?;
                    while let Some(batch) = reader.next_batch()? {
                        let index = &batch[0];
                        let mut visit = |key: u64| -> Result<bool, ExecutionError> {
                            match keys.entry(key) {
                                std::collections::hash_map::Entry::Occupied(entry) => {
                                    let count = &mut counts[*entry.get()].1;
                                    *count = count.checked_add(1).ok_or_else(|| {
                                        ExecutionError::Invalid("parallel GROUP BY count overflow".into())
                                    })?;
                                }
                                std::collections::hash_map::Entry::Vacant(entry) => {
                                    if retained.fetch_update(
                                        AtomicOrdering::AcqRel, AtomicOrdering::Acquire,
                                        |current| current.checked_add(1)
                                            .filter(|&total| total <= MAX_PARTIAL_KEYS),
                                    ).is_err() {
                                        return Ok(false);
                                    }
                                    entry.insert(counts.len());
                                    counts.push((key, 1));
                                }
                            }
                            Ok(true)
                        };
                        match (index, kind) {
                            (NumericColumn::UInt32(values), "UInt32") => {
                                for &value in values {
                                    if !visit(u64::from(value))? { return Ok(None); }
                                }
                            }
                            (NumericColumn::Int32(values), "Int32") => {
                                for &value in values {
                                    if !visit(value as i64 as u64)? { return Ok(None); }
                                }
                            }
                            (NumericColumn::UInt64(values), "UInt64") => {
                                for &value in values {
                                    if !visit(value)? { return Ok(None); }
                                }
                            }
                            (NumericColumn::Int64(values), "Int64") => {
                                for &value in values {
                                    if !visit(value as u64)? { return Ok(None); }
                                }
                            }
                            _ => return Err(ExecutionError::Invalid(
                                "parallel integer grouping encountered unexpected column".into(),
                            )),
                        }
                    }
                }
                Ok(Some(counts))
            })
        }).collect::<Vec<_>>();
        handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| {
            Err(ExecutionError::Invalid("parallel grouped worker panicked".into()))
        })).collect::<Vec<_>>()
    });
    let mut keys = HashMap::<u64, usize>::new();
    let mut counts = Vec::<(u64, u64)>::new();
    for chunk in chunks {
        let Some(chunk) = chunk? else { return Ok(None); };
        for (key, count) in chunk {
            match keys.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    let total = &mut counts[*entry.get()].1;
                    *total = total.checked_add(count)
                        .ok_or_else(|| ExecutionError::Invalid(
                            "parallel GROUP BY count overflow".into()
                        ))?;
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

pub(super) fn execute_grouped_part<W: Write>(
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
    let kind = metadata.columns.iter().find(|c| c.name == column)
        .map(|c| c.data_type.as_str());
    let ordering = query.order_by.as_ref().filter(|order|
        order.column == column && query.order_by_extra.is_empty());
    if let (Some(kind @ ("UInt32" | "Int32" | "UInt64" | "Int64")), Some(order)) = (kind, ordering) {
        let signed = matches!(kind, "Int32" | "Int64");
        let mut sorter = ExternalGroupCount64::new(signed, order.descending);
        while let Some(batch) = reader.next_batch()? {
            for row in 0..batch[0].len() {
                if !no_filters && !batch_matches(&batch, row, &comparisons) { continue; }
                let value = match &batch[0] {
                    NumericColumn::UInt32(values) => u64::from(values[row]),
                    NumericColumn::Int32(values) => values[row] as i64 as u64,
                    NumericColumn::UInt64(values) => values[row],
                    NumericColumn::Int64(values) => values[row] as u64,
                    _ => return Err(ExecutionError::Invalid(
                        "unexpected external GROUP BY key type".into())),
                };
                sorter.insert(value)?;
            }
        }
        writeln!(output, "{column}\t{}",
            query.alias.as_deref().unwrap_or("count"))?;
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
                ) { return Ok(true); }
            }
            if seen >= offset && emitted < limit {
                if signed {
                    writeln!(output, "{}\t{count}", key as i64)?;
                } else {
                    writeln!(output, "{key}\t{count}")?;
                }
                emitted += 1;
            }
            seen = seen.checked_add(1)
                .ok_or_else(|| ExecutionError::Invalid("GROUP BY row count overflow".into()))?;
            Ok(emitted < limit)
        })?;
        return Ok(());
    }
    if no_filters {
        if let Some(kind) = metadata.columns.iter()
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
    if no_filters
        && matches!(narrow, Some("UInt32" | "Int32" | "UInt64" | "Int64"))
    {
        if let Some(result) = parallel_integer_grouped_count(
            part, column, narrow.expect("integer group kind was validated"),
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
                        *count = count
                            .checked_add(1)
                            .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
                    }
                    Entry::Vacant(entry) => {
                        if groups.rows.len() >= MAX_GROUPS {
                            // GROUP BY without ORDER BY has no stable row-order
                            // contract. Spill and globally merge exact counts
                            // only when unique keys exceed the hash ceiling.
                            if query.order_by.is_none() {
                                return spill_unordered_groups(
                                    query, part, column, narrow.expect("validated 32/64-bit key"),
                                    &scan_columns, &comparisons, &metadata, output,
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
                                    query, part, column, narrow.expect("validated 32/64-bit key"),
                                    &scan_columns, &comparisons, &metadata, output,
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

pub(super) fn execute_grouped_numbers<W: Write>(
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

pub(super) fn execute_grouped_aggregate_numbers<W: Write>(
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
    if query.limit.is_none() && query.having.is_none()
        && query.order_by_extra.is_empty() && step != 0
    {
        if let Some(order) = query.order_by.as_ref().filter(|order| order.column == "number") {
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

pub(super) fn execute_count_column_part<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    // A non-nullable column has one value for every stored row. The
    // validated snapshot metadata already contains the exact row count.
    // Avoid opening and decompressing column streams for COUNT(column).
    if query.predicates.is_empty() && query.or_predicates.is_empty() {
        let selected = metadata
            .columns
            .iter()
            .find(|item| item.name == column)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown column: {column}")))?;
        if !selected.data_type.starts_with("Nullable(") {
            write_header(query, output, &format!("count({column})"))?;
            if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{}", metadata.rows)?;
            }
            return Ok(());
        }
    }
    let mut scan_columns = vec![column.to_owned()];
    let comparisons = compile_part_where(query, &mut scan_columns, |name, literal| {
        let col = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
        parse_numeric(&col.data_type, literal)
    })?;
    // All rows in a plain numeric column are non-NULL. For an unfiltered
    // COUNT(column), immutable part metadata is sufficient even for large tables.
    // Other types continue through the validated reader path.
    let fast_count_type = metadata.columns.iter().any(|candidate| {
        candidate.name == column
            && matches!(
                candidate.data_type.as_str(),
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
                    | "Float32"
                    | "Float64"
                    | "Date"
                    | "Date32"
                    | "DateTime"
            )
    });
    if fast_count_type && query.predicates.is_empty() && query.or_predicates.is_empty() {
        write_header(query, output, &format!("count({column})"))?;
        if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            writeln!(output, "{}", metadata.rows)?;
        }
        return Ok(());
    }
    let mut reader = NumericQueryReader::open_with_metadata(part, &scan_columns, &metadata)?;
    write_header(query, output, &format!("count({column})"))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let mut count = 0u64;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        if no_filters {
            count = count
                .checked_add(batch[0].len() as u64)
                .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
            continue;
        }
        // The batch counter evaluates simple typed comparisons directly
        // without boxing each numeric value or rechecking the predicate tree.
        count = count
            .checked_add(count_batch_matches(&batch, &comparisons) as u64)
            .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{count}")?;
    }
    Ok(())
}


/// Merge exact distinct-value bitmaps from disjoint physical part scans.
/// Unlike a hash set, memory is bounded by the 16-bit value domain and
/// independent of the number of input rows.
fn parallel_exact_narrow_distinct(
    part: &Path,
    scan_columns: &[String],
    comparisons: &[Vec<super::predicates::CompiledPredicate>],
    no_filters: bool,
    domain: usize,
) -> Result<Option<u64>, ExecutionError> {
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    let chunk_size = parts.len().div_ceil(workers);
    let chunks = std::thread::scope(|scope| {
        let handles = parts.chunks(chunk_size).map(|paths| {
            scope.spawn(move || -> Result<Vec<u64>, ExecutionError> {
                let mut bits = vec![0u64; domain / 64];
                for path in paths {
                    let mut reader = rusthouse_storage::NumericPartReader::open(path, scan_columns)?;
                    while let Some(batch) = reader.next_batch()? {
                        for row in 0..batch[0].len() {
                            if !no_filters && !batch_matches(&batch, row, comparisons) {
                                continue;
                            }
                            let slot = match &batch[0] {
                                NumericColumn::UInt8(values) => usize::from(values[row]),
                                NumericColumn::Int8(values) => usize::from(values[row] as u8),
                                NumericColumn::UInt16(values) => usize::from(values[row]),
                                NumericColumn::Int16(values) => usize::from(values[row] as u16),
                                _ => return Err(ExecutionError::Invalid(
                                    "unexpected narrow distinct column".into(),
                                )),
                            };
                            bits[slot / 64] |= 1u64 << (slot % 64);
                        }
                    }
                }
                Ok(bits)
            })
        }).collect::<Vec<_>>();
        handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| {
            Err(ExecutionError::Invalid("distinct worker panicked".into()))
        })).collect::<Vec<_>>()
    });
    let mut bits = vec![0u64; domain / 64];
    for chunk in chunks {
        for (dest, value) in bits.iter_mut().zip(chunk?) {
            *dest |= value;
        }
    }
    Ok(Some(
        bits.iter().map(|word| u64::from(word.count_ones())).sum(),
    ))
}

pub(super) fn execute_count_distinct_part<W: Write>(
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
    write_header(query, output, &format!("countDistinct({column})"))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    // Narrow integer domains fit in a tiny exact bitmap. This avoids
    // hashing every row of a billion-row low-cardinality column.
    let domain = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == column)
        .and_then(|candidate| match candidate.data_type.as_str() {
            "UInt8" | "Int8" => Some(256usize),
            "UInt16" | "Int16" => Some(65536usize),
            _ => None,
        });
    if query.order_by.is_none() && query.order_by_extra.is_empty() {
        if let Some(domain) = domain {
            if let Some(count) = parallel_exact_narrow_distinct(
                part, &scan_columns, &comparisons, no_filters, domain,
            )? {
                if query.offset.unwrap_or(0) == 0 {
                    writeln!(output, "{count}")?;
                }
                return Ok(());
            }
            let mut bits = vec![0u64; domain / 64];
            while let Some(batch) = reader.next_batch()? {
                // Narrow values remain in their native array. Apply WHERE
                // before setting the bit, without constructing NumberKey or
                // probing a hash table for each qualifying row.
                for row in 0..batch[0].len() {
                    if !no_filters && !batch_matches(&batch, row, &comparisons) {
                        continue;
                    }
                    let value = match &batch[0] {
                        NumericColumn::UInt8(values) => usize::from(values[row]),
                        NumericColumn::Int8(values) => usize::from(values[row] as u8),
                        NumericColumn::UInt16(values) => usize::from(values[row]),
                        NumericColumn::Int16(values) => usize::from(values[row] as u16),
                        _ => {
                            return Err(ExecutionError::Invalid(
                                "unexpected COUNT(DISTINCT) narrow column type".into(),
                            ));
                        }
                    };
                    bits[value / 64] |= 1u64 << (value % 64);
                }
            }
            if query.offset.unwrap_or(0) == 0 {
                let total: u64 = bits.iter().map(|word| u64::from(word.count_ones())).sum();
                writeln!(output, "{total}")?;
            }
            return Ok(());
        }
    }
    // Exact chunked bitmaps handle millions of 32-bit dimension IDs without
    // maintaining a large boxed HashSet per observed value.
    if query.order_by.is_none() && query.order_by_extra.is_empty()
        && metadata.columns.iter().any(|candidate| {
            candidate.name == column &&
                matches!(candidate.data_type.as_str(), "UInt32" | "Int32")
        })
    {
        let mut distinct = BitmapDistinct32::new();
        while let Some(batch) = reader.next_batch()? {
            for row in 0..batch[0].len() {
                if !no_filters && !batch_matches(&batch, row, &comparisons) {
                    continue;
                }
                let key = match &batch[0] {
                    NumericColumn::UInt32(values) => values[row],
                    NumericColumn::Int32(values) => values[row] as u32,
                    _ => return Err(ExecutionError::Invalid(
                        "unexpected 32-bit COUNT(DISTINCT) column type".into()
                    )),
                };
                distinct.insert(key)?;
            }
        }
        if query.offset.unwrap_or(0) == 0 {
            writeln!(output, "{}", distinct.len())?;
        }
        return Ok(());
    }
    let mut seen = HashSet::new();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !batch_matches(&batch, row, &comparisons) {
                continue;
            }
            let key = NumberKey::from(number_at(&batch[0], row));
            if seen.len() >= MAX_DISTINCT_ROWS {
                // At the memory cap, check membership without then inserting
                // the already-present key a second time.
                if !seen.contains(&key) {
                    return Err(ExecutionError::Invalid(
                        "COUNT(DISTINCT) supports at most 100000 unique values".into(),
                    ));
                }
            } else {
                seen.insert(key);
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{}", seen.len())?;
    }
    Ok(())
}

/// Update population/sample moments directly from a native numeric slot.
/// Matches AggregateState's Welford update order but skips Number dispatch.
fn update_native_moment(state: &mut AggregateState, numeric: f64) -> Result<(), ExecutionError> {
    let next_count = state.count.checked_add(1)
        .ok_or_else(|| ExecutionError::Invalid("aggregate row count overflow".into()))?;
    let difference = numeric - state.mean;
    state.mean += difference / next_count as f64;
    if state.function != AggregateFunction::Avg {
        state.squared_deviations += difference * (numeric - state.mean);
    }
    state.count = next_count;
    Ok(())
}

pub(super) fn execute_aggregate_part<W: Write>(
    query: &Select,
    part: &Path,
    function: AggregateFunction,
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
    write_header(query, output, &format!("{}({column})", function.name()))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    // Exact radix selection needs only 256 counters per byte and a bounded
    // number of repeated immutable scans, not a full materialized row array.
    if function == AggregateFunction::Median {
        if let Some(kind) = metadata.columns.iter().find(|candidate| candidate.name == column) {
            if super::radix_median::supports_type(&kind.data_type) {
                let median = if no_filters {
                    super::radix_median::median(part, column, &metadata)?
                } else {
                    super::radix_median::median_filtered(
                        part, column, &metadata, &scan_columns, &comparisons,
                    )?
                };
                if query.offset.unwrap_or(0) == 0 {
                    writeln!(output, "{}", median.unwrap_or_else(|| "NULL".into()))?;
                }
                return Ok(());
            }
        }
    }
    // For plain integer SUM without WHERE, operate on contiguous typed arrays
    // and avoid AggregateState dispatch and Number conversion on every row.
    let native_integer = metadata.columns.iter().any(|candidate| {
        candidate.name == column
            && matches!(
                candidate.data_type.as_str(),
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
    });
    if function == AggregateFunction::Sum && no_filters && native_integer {
        // Keep the accumulator in its native wide integer type, instead of
        // matching and rebuilding a Number enum for every input value.
        let mut unsigned_total = 0u128;
        let mut signed_total = 0i128;
        let mut saw_value = false;
        let mut signed_kind: Option<bool> = None;
        macro_rules! check_kind {
            ($signed:expr) => {{
                if signed_kind.is_some_and(|previous| previous != $signed) {
                    return Err(ExecutionError::Invalid("incompatible types in SUM".into()));
                }
                signed_kind = Some($signed);
            }};
        }
        macro_rules! fold_unsigned {
            ($values:expr) => {{
                check_kind!(false);
                for &value in $values {
                    saw_value = true;
                    unsigned_total = unsigned_total
                        .checked_add(u128::from(value))
                        .ok_or_else(|| ExecutionError::Invalid("unsigned SUM overflow".into()))?;
                }
            }};
        }
        macro_rules! fold_signed {
            ($values:expr) => {{
                check_kind!(true);
                for &value in $values {
                    saw_value = true;
                    signed_total = signed_total
                        .checked_add(i128::from(value))
                        .ok_or_else(|| ExecutionError::Invalid("signed SUM overflow".into()))?;
                }
            }};
        }
        while let Some(batch) = reader.next_batch()? {
            match &batch[0] {
                NumericColumn::UInt8(values) => fold_unsigned!(values),
                NumericColumn::UInt16(values) => fold_unsigned!(values),
                NumericColumn::UInt32(values) => fold_unsigned!(values),
                NumericColumn::UInt64(values) => fold_unsigned!(values),
                NumericColumn::UInt128(values) => fold_unsigned!(values),
                NumericColumn::Int8(values) => fold_signed!(values),
                NumericColumn::Int16(values) => fold_signed!(values),
                NumericColumn::Int32(values) => fold_signed!(values),
                NumericColumn::Int64(values) => fold_signed!(values),
                NumericColumn::Int128(values) => fold_signed!(values),
                _ => {
                    return Err(ExecutionError::Invalid(
                        "unexpected numeric SUM column type".into(),
                    ));
                }
            }
        }
        if query.offset.unwrap_or(0) == 0 {
            if !saw_value {
                writeln!(output, "NULL")?;
            } else if signed_kind == Some(true) {
                writeln!(output, "{signed_total}")?;
            } else {
                writeln!(output, "{unsigned_total}")?;
            }
        }
        return Ok(());
    }
    // Reduce each native integer batch to one MIN/MAX candidate.
    if matches!(function, AggregateFunction::Min | AggregateFunction::Max)
        && no_filters
        && native_integer
    {
        let mut extremum: Option<Number> = None;
        macro_rules! candidate {
            ($values:expr, $kind:ident, $wide:ty) => {{
                let next = if function == AggregateFunction::Min {
                    $values.iter().min()
                } else {
                    $values.iter().max()
                };
                if let Some(value) = next {
                    let value = Number::$kind(<$wide>::from(*value));
                    let replace = extremum.is_none_or(|previous| {
                        let ordering = compare_numbers(value, previous);
                        if function == AggregateFunction::Min {
                            ordering.is_lt()
                        } else {
                            ordering.is_gt()
                        }
                    });
                    if replace {
                        extremum = Some(value);
                    }
                }
            }};
        }
        while let Some(batch) = reader.next_batch()? {
            match &batch[0] {
                NumericColumn::UInt8(values) => candidate!(values, Unsigned, u128),
                NumericColumn::UInt16(values) => candidate!(values, Unsigned, u128),
                NumericColumn::UInt32(values) => candidate!(values, Unsigned, u128),
                NumericColumn::UInt64(values) => candidate!(values, Unsigned, u128),
                NumericColumn::UInt128(values) => candidate!(values, Unsigned, u128),
                NumericColumn::Int8(values) => candidate!(values, Signed, i128),
                NumericColumn::Int16(values) => candidate!(values, Signed, i128),
                NumericColumn::Int32(values) => candidate!(values, Signed, i128),
                NumericColumn::Int64(values) => candidate!(values, Signed, i128),
                NumericColumn::Int128(values) => candidate!(values, Signed, i128),
                _ => {
                    return Err(ExecutionError::Invalid(
                        "unexpected integer extremum column".into(),
                    ));
                }
            }
        }
        if query.offset.unwrap_or(0) == 0 {
            if let Some(value) = extremum {
                writeln!(output, "{value}")?;
            } else {
                writeln!(output, "NULL")?;
            }
        }
        return Ok(());
    }
    // AVG, variance and standard deviation on a billion non-nullable numeric
    // rows still require stable Welford arithmetic, but not an enum dispatch
    // or number conversion per row. Keep the exact update order.
    if no_filters && matches!(
        function,
        AggregateFunction::Avg
            | AggregateFunction::VarPop
            | AggregateFunction::VarSamp
            | AggregateFunction::StddevPop
            | AggregateFunction::StddevSamp
    ) {
        let mut state = AggregateState::new(function);
        macro_rules! fold_moments {
            ($values:expr) => {
                for &value in $values {
                    update_native_moment(&mut state, value as f64)?;
                }
            };
        }
        while let Some(batch) = reader.next_batch()? {
            match &batch[0] {
                NumericColumn::UInt8(values) => fold_moments!(values),
                NumericColumn::UInt16(values) => fold_moments!(values),
                NumericColumn::UInt32(values) => fold_moments!(values),
                NumericColumn::UInt64(values) => fold_moments!(values),
                NumericColumn::UInt128(values) => fold_moments!(values),
                NumericColumn::Int8(values) => fold_moments!(values),
                NumericColumn::Int16(values) => fold_moments!(values),
                NumericColumn::Int32(values) => fold_moments!(values),
                NumericColumn::Int64(values) => fold_moments!(values),
                NumericColumn::Int128(values) => fold_moments!(values),
                NumericColumn::Float32(values) => fold_moments!(values),
                NumericColumn::Float64(values) => fold_moments!(values),
            }
        }
        if query.offset.unwrap_or(0) == 0 {
            writeln!(output, "{}", state.render())?;
        }
        return Ok(());
    }
    let mut state = AggregateState::new(function);
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !batch_matches(&batch, row, &comparisons) {
                continue;
            }
            state.update(number_at(&batch[0], row))?;
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{}", state.render())?;
    }
    Ok(())
}

pub(super) fn execute_aggregate_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    function: AggregateFunction,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if column != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    let comparisons = compile_numbers_predicates(query)?;
    write_header(query, output, &format!("{}(number)", function.name()))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    // For an unfiltered arithmetic progression, these aggregates are exact
    // without visiting every row. Keep AVG iterative to preserve its rounding.
    if query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && matches!(
            function,
            AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max
        )
    {
        if query.offset.unwrap_or(0) == 0 {
            let result = if rows == 0 {
                "NULL".to_string()
            } else {
                let first = u128::from(start);
                let last = first + u128::from(rows - 1) * u128::from(step);
                match function {
                    AggregateFunction::Sum => ((first + last) * u128::from(rows) / 2).to_string(),
                    AggregateFunction::Min => first.to_string(),
                    AggregateFunction::Max => last.to_string(),
                    _ => unreachable!(),
                }
            };
            writeln!(output, "{result}")?;
        }
        return Ok(());
    }
    // A filtered arithmetic progression can be aggregated from its exact
    // matching interval without generating a billion individual rows.
    if matches!(
        function,
        AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max
    ) {
        if let Some((lower, upper)) = monotonic_range_indices(&comparisons, start, rows, step) {
            if query.offset.unwrap_or(0) == 0 {
                if lower == upper {
                    writeln!(output, "NULL")?;
                } else {
                    let first = u128::from(start) + u128::from(lower) * u128::from(step);
                    let last = u128::from(start) + u128::from(upper - 1) * u128::from(step);
                    let count = u128::from(upper - lower);
                    let result = match function {
                        // Halve before multiplying to avoid a transient u128
                        // overflow at the full UInt64 cardinality boundary.
                        AggregateFunction::Sum => {
                            let endpoints = first + last;
                            if count % 2 == 0 {
                                (count / 2) * endpoints
                            } else {
                                count * (endpoints / 2)
                            }
                        }
                        AggregateFunction::Min => first,
                        AggregateFunction::Max => last,
                        _ => unreachable!(),
                    };
                    writeln!(output, "{result}")?;
                }
            }
            return Ok(());
        }
    }
    // Positive equality/IN filters identify a small set of source positions.
    // Visit those exact rows instead of enumerating a billion unmatched values.
    if let Some(indices) = sparse_generated_positions(&comparisons, start, rows, step) {
        let mut state = AggregateState::new(function);
        for index in indices {
            let value = u128::from(start) + u128::from(index) * u128::from(step);
            state.update(Number::Unsigned(value))?;
        }
        if query.offset.unwrap_or(0) == 0 {
            writeln!(output, "{}", state.render())?;
        }
        return Ok(());
    }
    // Exclude a small finite set from an arithmetic progression in O(k).
    if matches!(
        function,
        AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max
    ) {
        if let Some(excluded) = excluded_generated_positions(&comparisons, start, rows, step) {
            if query.offset.unwrap_or(0) == 0 {
                let remaining = rows - excluded.len() as u64;
                if remaining == 0 {
                    writeln!(output, "NULL")?;
                } else {
                    let first = u128::from(start);
                    let stride = u128::from(step);
                    let last = first + u128::from(rows - 1) * stride;
                    let result = match function {
                        AggregateFunction::Sum => {
                            let endpoints = first + last;
                            let baseline = if rows % 2 == 0 {
                                u128::from(rows / 2) * endpoints
                            } else {
                                u128::from(rows) * (endpoints / 2)
                            };
                            let removed = excluded
                                .iter()
                                .map(|&index| first + u128::from(index) * stride)
                                .sum::<u128>();
                            baseline.checked_sub(removed).ok_or_else(|| {
                                ExecutionError::Invalid("excluded SUM underflow".into())
                            })?
                        }
                        AggregateFunction::Min => {
                            let mut index = 0u64;
                            while excluded.binary_search(&index).is_ok() {
                                index += 1;
                            }
                            first + u128::from(index) * stride
                        }
                        AggregateFunction::Max => {
                            let mut index = rows - 1;
                            while excluded.binary_search(&index).is_ok() {
                                index -= 1;
                            }
                            first + u128::from(index) * stride
                        }
                        _ => unreachable!(),
                    };
                    writeln!(output, "{result}")?;
                }
            }
            return Ok(());
        }
    }
    let mut state = AggregateState::new(function);
    for index in 0..rows {
        let number = start + index * step;
        let value = Number::Unsigned(u128::from(number));
        if !number_matches_all(value, &comparisons) {
            continue;
        }
        state.update(value)?;
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{}", state.render())?;
    }
    Ok(())
}

#[cfg(test)]
mod native_moment_tests {
    use super::*;

    #[test]
    fn native_welford_matches_generic_aggregates_for_large_signed_values() {
        let inputs = [
            Number::Signed(-91),
            Number::Unsigned(u128::from(u64::MAX)),
            Number::Float(1.75),
            Number::Float(-2.5),
            Number::Unsigned(7),
        ];
        for function in [
            AggregateFunction::Avg,
            AggregateFunction::VarPop,
            AggregateFunction::VarSamp,
            AggregateFunction::StddevPop,
            AggregateFunction::StddevSamp,
        ] {
            let mut native = AggregateState::new(function);
            let mut baseline = AggregateState::new(function);
            for value in inputs {
                let numeric = match value {
                    Number::Signed(v) => v as f64,
                    Number::Unsigned(v) => v as f64,
                    Number::Float(v) => v,
                };
                update_native_moment(&mut native, numeric).unwrap();
                baseline.update(value).unwrap();
            }
            assert_eq!(native.mean.to_bits(), baseline.mean.to_bits());
            assert_eq!(native.squared_deviations.to_bits(), baseline.squared_deviations.to_bits());
            assert_eq!(native.render(), baseline.render());
        }
    }
}
