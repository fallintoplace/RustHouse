use super::predicates::CompiledPredicate;
use super::*;
mod dense_grouped;
mod grouped;
pub(super) use grouped::{
    execute_grouped_aggregate_numbers, execute_grouped_numbers, execute_grouped_part,
};
use self::dense_grouped::*;

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
    let workers = super::query_workers(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    // Claim large and small frozen parts dynamically to balance skew.
    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let chunks = std::thread::scope(|scope| -> Result<_, ExecutionError> {
        let handles = (0..workers)
            .map(|_| {
                let cursor = &cursor;
                let parts = &parts;
                std::thread::Builder::new()
                    .spawn_scoped(scope, move || -> Result<Vec<u64>, ExecutionError> {
                        let mut bits = vec![0u64; domain / 64];
                        while let Some(path) =
                            parts.get(cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                        {
                            let mut reader =
                                rusthouse_storage::NumericPartReader::open(path, scan_columns)?;
                            while let Some(batch) = reader.next_batch()? {
                                for row in 0..batch[0].len() {
                                    if !no_filters && !batch_matches(&batch, row, comparisons) {
                                        continue;
                                    }
                                    let slot = match &batch[0] {
                                        NumericColumn::UInt8(values) => usize::from(values[row]),
                                        NumericColumn::Int8(values) => {
                                            usize::from(values[row] as u8)
                                        }
                                        NumericColumn::UInt16(values) => usize::from(values[row]),
                                        NumericColumn::Int16(values) => {
                                            usize::from(values[row] as u16)
                                        }
                                        _ => {
                                            return Err(ExecutionError::Invalid(
                                                "unexpected narrow distinct column".into(),
                                            ));
                                        }
                                    };
                                    bits[slot / 64] |= 1u64 << (slot % 64);
                                }
                            }
                        }
                        Ok(bits)
                    })
                    .map_err(|error| {
                        ExecutionError::Invalid(format!(
                            "cannot start narrow DISTINCT worker: {error}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(ExecutionError::Invalid("distinct worker panicked".into()))
                })
            })
            .collect::<Vec<_>>())
    })?;
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
                part,
                &scan_columns,
                &comparisons,
                no_filters,
                domain,
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
    if query.order_by.is_none()
        && query.order_by_extra.is_empty()
        && metadata.columns.iter().any(|candidate| {
            candidate.name == column && matches!(candidate.data_type.as_str(), "UInt32" | "Int32")
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
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unexpected 32-bit COUNT(DISTINCT) column type".into(),
                        ));
                    }
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
    let next_count = state
        .count
        .checked_add(1)
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
        if let Some(kind) = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == column)
        {
            if super::radix_median::supports_type(&kind.data_type) {
                let median = if no_filters {
                    super::radix_median::median(part, column, &metadata)?
                } else {
                    super::radix_median::median_filtered(
                        part,
                        column,
                        &metadata,
                        &scan_columns,
                        &comparisons,
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
    if no_filters
        && matches!(
            function,
            AggregateFunction::Avg
                | AggregateFunction::VarPop
                | AggregateFunction::VarSamp
                | AggregateFunction::StddevPop
                | AggregateFunction::StddevSamp
        )
    {
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
            assert_eq!(
                native.squared_deviations.to_bits(),
                baseline.squared_deviations.to_bits()
            );
            assert_eq!(native.render(), baseline.render());
        }
    }
}
