//! Isolate the generated-series execution path from the main query executor.

use super::*;

pub(super) fn execute_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    part: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if part.is_some() {
        return Err(ExecutionError::Invalid(
            "--part cannot be used with numbers(N)".into(),
        ));
    }
    // Protect large virtual sources from full scans while allowing exact
    // arithmetic aggregates and direct pages over generated unique keys.
    if rows > 10_000_000 {
        let scalar_count = matches!(
            query.projection,
            Projection::Count
                | Projection::CountColumn { .. }
                | Projection::CountDistinct { .. }
                | Projection::ApproxDistinct { .. }
                | Projection::Histogram { .. }
        );
        let simple_result = query.order_by.is_none()
            && query.order_by_extra.is_empty()
            && query.having.is_none()
            && !query.distinct;
        let arithmetic_count = if query.predicates.is_empty() && query.or_predicates.is_empty() {
            true
        } else if scalar_count && simple_result {
            let compiled = compile_numbers_predicates(query)?;
            count_monotonic_numbers(&compiled, start, rows, step).is_some()
        } else {
            false
        };
        // Bounded pages are also safe when we can jump directly to the
        // requested generated values instead of scanning the full input.
        let bounded_page = matches!(query.projection, Projection::All | Projection::Columns(_))
            && query.limit.is_some_and(|limit| limit <= MAX_SORTED_ROWS)
            && query.having.is_none()
            && !query.distinct
            && query.order_by_extra.is_empty()
            && ((query.predicates.is_empty() && query.or_predicates.is_empty())
                || monotonic_range_indices(&compile_numbers_predicates(query)?, start, rows, step)
                    .is_some());
        // A GROUP BY number page has one row per key and can jump directly
        // to the requested offset without materializing earlier groups.
        let grouped_page = (matches!(
            &query.projection,
            Projection::GroupedCount { column } if column == "number"
        ) || matches!(
            &query.projection,
            Projection::GroupedAggregate { column, value_column, .. }
                if column == "number" && value_column == "number"
        ) || matches!(
            &query.projection,
            Projection::GroupedAggregates { column, items, filters, .. }
                if column == "number" && items.iter().all(|(_, key)| key == "number")
                    && filters.iter().all(Option::is_none)
        )) && query.limit.is_some_and(|limit| limit <= MAX_SORTED_ROWS)
            && ((query.order_by.is_none() && simple_result)
                || (query
                    .order_by
                    .as_ref()
                    .is_some_and(|order| order.column == "number")
                    && query.having.is_none()
                    && !query.distinct
                    && query.order_by_extra.is_empty()
                    && (matches!(&query.projection,
                        Projection::GroupedCount { column } if column == "number"
                    ) || matches!(&query.projection,
                        Projection::GroupedAggregate { column, value_column, .. }
                            if column == "number" && value_column == "number"
                    ) || matches!(&query.projection,
                        Projection::GroupedAggregates { column, items, filters, .. }
                            if column == "number" && items.iter().all(|(_, key)| key == "number")
                                && filters.iter().all(Option::is_none)
                    ))))
            && ((query.predicates.is_empty() && query.or_predicates.is_empty())
                || monotonic_range_indices(&compile_numbers_predicates(query)?, start, rows, step)
                    .is_some());
        // Only admit aggregates with proven O(1) or bounded sparse fast paths.
        // Other aggregate functions still require the normal scan bound.
        let direct_aggregate = if let Projection::Aggregate { function, column } = &query.projection
        {
            column == "number"
                && matches!(
                    function,
                    AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max
                )
                && simple_result
                && (if query.predicates.is_empty() && query.or_predicates.is_empty() {
                    true
                } else {
                    let compiled = compile_numbers_predicates(query)?;
                    monotonic_range_indices(&compiled, start, rows, step).is_some()
                        || sparse_generated_positions(&compiled, start, rows, step).is_some()
                        || excluded_generated_positions(&compiled, start, rows, step).is_some()
                })
        } else {
            false
        };
        let bounded_multi_aggregate = matches!(
            &query.projection,
            Projection::MultipleAggregates(items)
                if multi_aggregate::supports_generated_series_metrics(items)
        ) && simple_result
            && ((query.predicates.is_empty() && query.or_predicates.is_empty())
                || sparse_generated_positions(
                    &compile_numbers_predicates(query)?,
                    start,
                    rows,
                    step,
                )
                .is_some());
        if !(bounded_page
            || grouped_page
            || direct_aggregate
            || bounded_multi_aggregate
            || (scalar_count && simple_result && arithmetic_count))
        {
            return Err(ExecutionError::Invalid(
                "virtual inputs over 10000000 rows require an exact aggregate or bounded page query".into(),
            ));
        }
    }
    if let Projection::MultipleAggregates(items) = &query.projection {
        return multi_aggregate::execute_numbers(query, start, rows, step, items, output);
    }
    if let Projection::GroupedAggregates {
        column,
        items,
        aliases,
        filters,
    } = &query.projection
    {
        if items
            .iter()
            .any(|(function, _)| *function == AggregateFunction::CountDistinct)
        {
            return Err(ExecutionError::Invalid(
                "mixed grouped COUNT(DISTINCT) requires a stored table".into(),
            ));
        }
        if query.grouped_expressions.iter().any(Option::is_some) {
            return Err(ExecutionError::Invalid(
                "generated grouped expressions require a stored table".into(),
            ));
        }
        // Generated metrics do not yet implement per-metric FILTER. Never
        // silently ignore a parsed predicate and return incorrect results.
        if filters.iter().any(Option::is_some) {
            return Err(ExecutionError::Invalid(
                "generated grouped FILTER metrics require a stored table".into(),
            ));
        }
        return grouped_generated::execute(
            query, start, rows, step, column, items, aliases, output,
        );
    }
    if matches!(
        query.projection,
        Projection::GroupedAggregates { .. }
            | Projection::GroupedMultiAggregates { .. }
            | Projection::GroupedDistinctCount { .. }
            | Projection::GroupedMultiDistinctCount { .. }
    ) {
        return Err(ExecutionError::Invalid(
            "multiple aggregates require a stored table".into(),
        ));
    }
    if let Projection::CountColumn { column } = &query.projection {
        if column != "number" {
            return Err(ExecutionError::Invalid(
                "numbers(N) exposes only the number column".into(),
            ));
        }
        let comparisons = compile_numbers_predicates(query)?;
        write_header(query, output, "count(number)")?;
        if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            let count = if query.predicates.is_empty() && query.or_predicates.is_empty() {
                rows
            } else {
                count_monotonic_numbers(&comparisons, start, rows, step).unwrap_or_else(|| {
                    let mut count = 0u64;
                    for index in 0..rows {
                        let number = start + index * step;
                        if number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
                            count += 1;
                        }
                    }
                    count
                })
            };
            writeln!(output, "{count}")?;
        }
        return Ok(());
    }
    if matches!(query.projection, Projection::ApproxDistinctState { .. }) {
        return Err(ExecutionError::Invalid(
            "uniqState requires a physical table snapshot".into(),
        ));
    }
    if let Projection::CountDistinct { column } | Projection::ApproxDistinct { column } =
        &query.projection
    {
        if column != "number" {
            return Err(ExecutionError::Invalid(
                "numbers(N) exposes only the number column".into(),
            ));
        }
        let comparisons = compile_numbers_predicates(query)?;
        let header = if matches!(query.projection, Projection::ApproxDistinct { .. }) {
            "uniq(number)"
        } else {
            "countDistinct(number)"
        };
        write_header(query, output, header)?;
        if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            // Generated numbers are unique, so unfiltered COUNT(DISTINCT) is their row count.
            let count = if query.predicates.is_empty() && query.or_predicates.is_empty() {
                rows
            } else {
                count_monotonic_numbers(&comparisons, start, rows, step).unwrap_or_else(|| {
                    let mut matched = 0u64;
                    for index in 0..rows {
                        let number = start + index * step;
                        if number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
                            matched += 1;
                        }
                    }
                    matched
                })
            };
            writeln!(output, "{count}")?;
        }
        return Ok(());
    }
    if matches!(query.projection, Projection::Histogram { .. }) {
        return histogram::execute_numbers(query, start, rows, step, output);
    }
    if let Projection::Aggregate { function, column } = &query.projection {
        return execute_aggregate_numbers(query, start, rows, step, *function, column, output);
    }
    if let Projection::GroupedCount { column } = &query.projection {
        return execute_grouped_numbers(query, start, rows, step, column, output);
    }
    if matches!(query.projection, Projection::GroupedAggregate { .. }) {
        return execute_grouped_aggregate_numbers(query, start, rows, step, output);
    }
    let projected: Vec<&str> = match &query.projection {
        Projection::All => vec!["number"],
        Projection::Columns(names) => {
            if names.iter().any(|name| name != "number") {
                return Err(ExecutionError::Invalid(
                    "numbers(N) exposes only the number column".into(),
                ));
            }
            names.iter().map(String::as_str).collect()
        }
        Projection::Count => Vec::new(),
        Projection::Aggregate { .. }
        | Projection::CountColumn { .. }
        | Projection::CountDistinct { .. }
        | Projection::ApproxDistinct { .. }
        | Projection::Histogram { .. }
        | Projection::ApproxDistinctState { .. }
        | Projection::GroupedCount { .. }
        | Projection::GroupedDistinctCount { .. }
        | Projection::GroupedMultiDistinctCount { .. }
        | Projection::GroupedMultiCount { .. }
        | Projection::GroupedAggregate { .. }
        | Projection::GroupedAggregates { .. }
        | Projection::GroupedMultiAggregates { .. }
        | Projection::MultipleAggregates(_)
        | Projection::StringLength { .. }
        | Projection::Literal(_)
        | Projection::Constants(_) => unreachable!(),
    };
    let comparisons = compile_numbers_predicates(query)?;
    let is_count = matches!(query.projection, Projection::Count);
    let mut sorting = query
        .order_by
        .as_ref()
        .map(|order| {
            if query.order_source_column(&order.column) != "number" {
                return Err(ExecutionError::Invalid(
                    "numbers(N) exposes only the number column".into(),
                ));
            }
            SortedRows::with_offset(query.limit, query.offset.unwrap_or(0), order.descending)
        })
        .transpose()?;
    if query
        .order_by_extra
        .iter()
        .any(|order| query.order_source_column(&order.column) != "number")
    {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    if is_count {
        write_header(query, output, "count")?;
        if query.predicates.is_empty() && query.or_predicates.is_empty() {
            if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
                writeln!(output, "{rows}")?;
            }
            return Ok(());
        }
        if sorting.is_none() && query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
            if let Some(count) = count_monotonic_numbers(&comparisons, start, rows, step) {
                writeln!(output, "{count}")?;
                return Ok(());
            }
        }
    } else {
        write_header(query, output, &projected.join("\t"))?;
    }
    if query.limit == Some(0) {
        return Ok(());
    }

    // All generated values are unique and monotonic. With a simple range
    // predicate, both the first matching index and the page boundary are
    // known without visiting any of the preceding billion generated rows.
    if !is_count && (!query.predicates.is_empty() || !query.or_predicates.is_empty()) {
        if let Some(limit) = query.limit {
            if let Some((lower, upper)) = monotonic_range_indices(&comparisons, start, rows, step) {
                let available = upper - lower;
                let skip = (query.offset.unwrap_or(0) as u64).min(available);
                let emit = (limit as u64).min(available - skip);
                let descending = sorting.as_ref().is_some_and(SortedRows::descending);
                for position in skip..skip + emit {
                    let index = if descending {
                        upper - 1 - position
                    } else {
                        lower + position
                    };
                    let number = u128::from(start) + u128::from(index) * u128::from(step);
                    for column in 0..projected.len() {
                        if column > 0 {
                            output.write_all(b"\t")?;
                        }
                        write!(output, "{number}")?;
                    }
                    writeln!(output)?;
                }
                return Ok(());
            }
        }
    }

    // numbers() is already sorted and its values are unique. A filter-free
    // ORDER BY can page the generated range directly rather than filling a heap.
    let unfiltered_sort = if query.predicates.is_empty() && query.or_predicates.is_empty() {
        sorting.as_ref()
    } else {
        None
    };
    if let Some(sorted) = unfiltered_sort {
        if rows > MAX_SORTED_ROWS as u64 && sorted.requested() > MAX_SORTED_ROWS {
            return Err(ExecutionError::Invalid(format!(
                "ORDER BY requires LIMIT <= {MAX_SORTED_ROWS} for large results"
            )));
        }
        let total = usize::try_from(rows).unwrap_or(usize::MAX);
        let begin = sorted.offset().min(total);
        let end = sorted
            .offset()
            .saturating_add(query.limit.unwrap_or(usize::MAX))
            .min(total);
        for position in begin..end {
            let index = if sorted.descending() {
                rows - position as u64 - 1
            } else {
                position as u64
            };
            let number = start + index * step;
            for column in 0..projected.len() {
                if column != 0 {
                    write!(output, "\t")?;
                }
                write!(output, "{number}")?;
            }
            writeln!(output)?;
        }
        return Ok(());
    }

    // A filtered generated sequence is still ordered by number. For a
    // bounded ORDER BY page, scan in the requested direction and stop after
    // the page is full instead of building a heap from every matching row.
    // Keep the existing bounded-sort limit and unbounded-sort behavior.
    if !is_count {
        if let (Some(sorted), Some(limit)) = (sorting.as_ref(), query.limit) {
            if sorted.requested() <= MAX_SORTED_ROWS {
                let mut remaining_offset = sorted.offset();
                let mut emitted = 0usize;
                for position in 0..rows {
                    let index = if sorted.descending() {
                        rows - 1 - position
                    } else {
                        position
                    };
                    let number = start + index * step;
                    if !number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
                        continue;
                    }
                    if remaining_offset > 0 {
                        remaining_offset -= 1;
                        continue;
                    }
                    for column in 0..projected.len() {
                        if column > 0 {
                            output.write_all(b"\t")?;
                        }
                        write!(output, "{number}")?;
                    }
                    writeln!(output)?;
                    emitted += 1;
                    if emitted == limit {
                        break;
                    }
                }
                return Ok(());
            }
        }
    }

    let mut matched = 0u64;
    let mut emitted = 0usize;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(usize::MAX);
    // Generated rows are unique and ordered. A filter-free scan can start at OFFSET.
    let fast_skip = if !is_count
        && sorting.is_none()
        && query.predicates.is_empty()
        && query.or_predicates.is_empty()
    {
        offset.min(rows as usize)
    } else {
        0
    };
    let mut skipped = fast_skip;
    for index in fast_skip as u64..rows {
        let number = start + index * step;
        if !number_matches_all(Number::Unsigned(u128::from(number)), &comparisons) {
            continue;
        }
        if let Some(sorted) = sorting.as_mut() {
            let value = Number::Unsigned(u128::from(number));
            sorted.push(value, vec![value; projected.len()])?;
            continue;
        }
        if is_count {
            matched += 1;
        } else {
            if skipped < offset {
                skipped += 1;
                continue;
            }
            for (index, _) in projected.iter().enumerate() {
                if index != 0 {
                    write!(output, "\t")?;
                }
                write!(output, "{number}")?;
            }
            writeln!(output)?;
            emitted += 1;
            if emitted == limit {
                break;
            }
        }
    }
    if let Some(sorted) = sorting {
        return sorted.write(output);
    }
    if is_count && query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{matched}")?;
    }
    Ok(())
}
