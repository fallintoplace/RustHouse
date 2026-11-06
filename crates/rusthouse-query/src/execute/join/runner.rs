//! Separate the JOIN execution flow from hash lookup and streaming helpers.

use super::*;

pub fn execute_join<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if query.on_right_like.is_some()
        && (query.kind != JoinType::Left
            || query.grouped_count_column.is_none()
            || !query.steps.is_empty())
    {
        return Err(invalid(
            "JOIN ON LIKE requires grouped LEFT JOIN COUNT(column)",
        ));
    }
    if query.grouped_count_column.is_some()
        && (query
            .grouped_count
            .as_ref()
            .is_none_or(|keys| keys.len() != 1)
            || !matches!(query.kind, JoinType::Inner | JoinType::Left)
            || !query.steps.is_empty()
            || query.aggregate.is_some()
            || query.grouped_sum.is_some()
            || query.grouped_distinct.is_some()
            || !query.grouped_case_counts.is_empty()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
            || query.scalar_where.is_some()
            || !query.where_comparisons_extra.is_empty()
            || !query.where_column_comparisons.is_empty()
            || !query.where_in_filters.is_empty()
            || !query.where_like_filters.is_empty())
    {
        return Err(invalid(
            "grouped COUNT(column) requires one key and two INNER or LEFT inputs",
        ));
    }
    if query
        .grouped_sum_having
        .as_ref()
        .is_some_and(|having| having.subquery.is_some())
    {
        return Err(invalid(
            "JOIN HAVING subquery must be bound before execution",
        ));
    }
    if query.in_subquery.is_some() {
        return Err(invalid("JOIN IN subquery must be bound before execution"));
    }
    if query.scalar_sum.is_some()
        || query.scalar_formula.is_some()
        || query.scalar_where.is_some()
        || query.grouped_distinct.is_some()
        || query.grouped_sum.is_some()
        || query.grouped_min.is_some()
        || query.grouped_formula.is_some()
        || query.grouped_year.is_some()
        || (!query.grouped_case_counts.is_empty() && query.kind == crate::JoinType::Full)
        || !query.grouped_key_aliases.is_empty()
    {
        return super::super::multi_join::execute_multi_join(
            query,
            &[left_path.to_path_buf(), right_path.to_path_buf()],
            output,
        );
    }
    let left_meta = inspect_query_source(left_path)?;
    let right_meta = inspect_query_source(right_path)?;
    // Grouped CASE aggregates do not depend on input order. Stream the large
    // side when the other side fits the existing bounded hash build. ANY and
    // USING retain their original ordering and projection semantics.
    if !query.grouped_case_counts.is_empty()
        && query.kind == JoinType::Inner
        && !query.any
        && !query.left_alias.eq_ignore_ascii_case(&query.right_alias)
        && query.using_keys.is_empty()
        && query.columns.is_some()
        && query.steps.is_empty()
        && right_meta.rows > MAX_STREAM_BUILD_ROWS
        && left_meta.rows <= MAX_STREAM_BUILD_ROWS
    {
        let mut swapped = query.clone();
        std::mem::swap(&mut swapped.left, &mut swapped.right);
        std::mem::swap(&mut swapped.left_alias, &mut swapped.right_alias);
        return execute_join(&swapped, right_path, left_path, output);
    }
    let left_names = left_meta
        .columns
        .iter()
        .map(|col| col.name.clone())
        .collect::<Vec<_>>();
    let right_names = right_meta
        .columns
        .iter()
        .map(|col| col.name.clone())
        .collect::<Vec<_>>();
    let width = left_names.len();
    let selected: Vec<(usize, String, Option<usize>)> = if let Some(columns) = &query.columns {
        let mut selected = Vec::new();
        for col in columns {
            if col.name == "*" {
                let (start, names) = if col.relation == query.left_alias {
                    (0, &left_names)
                } else if col.relation == query.right_alias {
                    (width, &right_names)
                } else {
                    return Err(invalid(format!("unknown JOIN qualifier: {}", col.relation)));
                };
                selected.extend(names.iter().enumerate().map(|(index, name)| {
                    (start + index, format!("{}.{}", col.relation, name), None)
                }));
            } else {
                let index = resolve(query, col, &left_names, &right_names)?;
                let label = if col.relation.is_empty() {
                    col.name.clone()
                } else {
                    col.name.clone()
                };
                selected.push((index, label, None));
            }
        }
        selected
    } else {
        let mut selected = Vec::new();
        // SQL USING projects shared keys first, in USING list order.
        // The fallback preserves unmatched right keys for RIGHT/FULL joins.
        for name in &query.using_keys {
            let left_index = left_names
                .iter()
                .position(|column| column == name)
                .ok_or_else(|| {
                    invalid(format!("unknown JOIN column: {}.{name}", query.left_alias))
                })?;
            let right_index = right_names
                .iter()
                .position(|column| column == name)
                .ok_or_else(|| {
                    invalid(format!("unknown JOIN column: {}.{name}", query.right_alias))
                })?;
            selected.push((
                left_index,
                format!("{}.{}", query.left_alias, name),
                Some(width + right_index),
            ));
        }
        for (index, name) in left_names.iter().enumerate() {
            if !query.using_keys.contains(name) {
                selected.push((index, format!("{}.{}", query.left_alias, name), None));
            }
        }
        for (index, name) in right_names.iter().enumerate() {
            if !query.using_keys.contains(name) {
                selected.push((
                    width + index,
                    format!("{}.{}", query.right_alias, name),
                    None,
                ));
            }
        }
        selected
    };
    let order = query
        .order_by
        .iter()
        .map(|(column, descending, nulls_first)| {
            if column.is_count_order()
                && (query.grouped_count.is_some()
                    || query.aggregate == Some(JoinAggregate::CountStar))
            {
                Ok((usize::MAX, *descending, *nulls_first))
            } else {
                resolve(query, column, &left_names, &right_names)
                    .map(|index| (index, *descending, *nulls_first))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let null_filter = query
        .where_null
        .as_ref()
        .map(|(column, not_null)| {
            resolve(query, column, &left_names, &right_names).map(|index| (index, *not_null))
        })
        .transpose()?;
    let mut left_keys = Vec::new();
    let mut right_keys = Vec::new();
    for (first, second) in &query.keys {
        let first_index = resolve(query, first, &left_names, &right_names)?;
        let second_index = resolve(query, second, &left_names, &right_names)?;
        let (left_index, right_index) = if first_index < width && second_index >= width {
            (first_index, second_index - width)
        } else if second_index < width && first_index >= width {
            (second_index, first_index - width)
        } else {
            return Err(invalid("JOIN ON must compare one column from each table"));
        };
        let left_type = &left_meta.columns[left_index].data_type;
        let right_type = &right_meta.columns[right_index].data_type;
        if !compatible_join_types(left_type, right_type) {
            return Err(invalid(format!(
                "JOIN key types differ: {left_type} and {right_type}"
            )));
        }
        left_keys.push(left_index);
        right_keys.push(right_index);
    }

    if !query.grouped_case_counts.is_empty()
        || ((query.where_comparison.is_some()
            || !query.where_column_comparisons.is_empty()
            || !query.where_in_filters.is_empty()
            || !query.where_like_filters.is_empty())
            && query.grouped_count.is_none())
    {
        return comparison_filter::execute(
            query,
            left_path,
            right_path,
            &left_names,
            &right_names,
            &selected,
            &left_keys,
            &right_keys,
            output,
        );
    }

    if query.grouped_count.is_some() {
        return grouped::execute(
            query,
            left_path,
            right_path,
            &left_names,
            &right_names,
            &left_keys,
            &right_keys,
            null_filter,
            output,
        );
    }

    if let Some(JoinAggregate::CountColumn(column)) = &query.aggregate {
        return count_column::execute(
            query,
            left_path,
            right_path,
            &left_names,
            &right_names,
            &left_keys,
            &right_keys,
            null_filter,
            column,
            output,
        );
    }

    if let Some(JoinAggregate::Numeric { function, column }) = &query.aggregate {
        return aggregate::execute_numeric(
            query,
            left_path,
            right_path,
            &left_names,
            &right_names,
            &left_keys,
            &right_keys,
            null_filter,
            *function,
            column,
            output,
        );
    }

    if query.aggregate == Some(JoinAggregate::CountStar) && query.kind == JoinType::Inner {
        return execute_streaming_inner_count(
            query,
            left_path,
            right_path,
            InnerJoinColumns {
                left_names: &left_names,
                right_names: &right_names,
                selected: &selected,
                left_keys: &left_keys,
                right_keys: &right_keys,
                null_filter,
            },
            left_meta.rows,
            right_meta.rows,
            output,
        );
    }

    // Scalar COUNT LIMIT 0 has a header-only result, including OUTER joins.
    if query.aggregate == Some(JoinAggregate::CountStar) && query.limit == Some(0) {
        writeln!(
            output,
            "{}",
            query.aggregate_alias.as_deref().unwrap_or("count")
        )?;
        return Ok(());
    }
    // LIMIT 0 needs the validated output schema, but not either input scan.
    if query.limit == Some(0) {
        let header = selected_header(&selected);
        writeln!(output, "{header}")?;
        return Ok(());
    }

    // A validated INNER join with an empty input cannot produce matches.
    if query.kind == JoinType::Inner && (left_meta.rows == 0 || right_meta.rows == 0) {
        let header = selected_header(&selected);
        writeln!(output, "{header}")?;
        return Ok(());
    }

    if query.aggregate.is_none()
        && query.kind == JoinType::Right
        && order.is_empty()
        && left_meta.rows <= MAX_STREAM_BUILD_ROWS
        && right_meta.rows > MAX_INPUT_ROWS
    {
        return execute_streaming_right_outer(
            query,
            left_path,
            right_path,
            InnerJoinColumns {
                left_names: &left_names,
                right_names: &right_names,
                selected: &selected,
                left_keys: &left_keys,
                right_keys: &right_keys,
                null_filter,
            },
            output,
        );
    }

    if query.kind == JoinType::Inner && order.is_empty() {
        // A very large right table can be the streamed side if the left fits
        // the same bounded hash-build budget.
        if prefer_left_hash_build(left_meta.rows, right_meta.rows, query.any)
            || (query.any
                && right_meta.rows > MAX_STREAM_BUILD_ROWS
                && left_meta.rows <= MAX_STREAM_BUILD_ROWS)
        {
            return execute_streaming_right(
                query,
                left_path,
                right_path,
                InnerJoinColumns {
                    left_names: &left_names,
                    right_names: &right_names,
                    selected: &selected,
                    left_keys: &left_keys,
                    right_keys: &right_keys,
                    null_filter,
                },
                output,
            );
        }
        return execute_streaming_inner(
            query,
            left_path,
            right_path,
            InnerJoinColumns {
                left_names: &left_names,
                right_names: &right_names,
                selected: &selected,
                left_keys: &left_keys,
                right_keys: &right_keys,
                null_filter,
            },
            output,
        );
    }

    if query.kind == JoinType::Inner
        && !order.is_empty()
        && left_meta.rows > MAX_INPUT_ROWS
        && right_meta.rows <= MAX_STREAM_BUILD_ROWS
    {
        if let Some(requested) = query
            .limit
            .and_then(|limit| query.offset.checked_add(limit))
            .filter(|&requested| requested <= MAX_RESULT_ROWS)
        {
            return execute_streaming_ordered_inner(
                query,
                left_path,
                right_path,
                InnerJoinColumns {
                    left_names: &left_names,
                    right_names: &right_names,
                    selected: &selected,
                    left_keys: &left_keys,
                    right_keys: &right_keys,
                    null_filter,
                },
                &order,
                requested,
                output,
            );
        }
    }

    let left_rows = read_rows(
        left_path,
        &left_names,
        left_meta.rows,
        MAX_INPUT_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    let right_rows = read_rows(
        right_path,
        &right_names,
        right_meta.rows,
        MAX_INPUT_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    let mut hash = JoinHash::with_capacity(
        right_keys.len(),
        initial_hash_capacity(right_rows.len()),
        single_numeric_join_key(&right_rows, &right_keys),
    );
    let mut build_key = Vec::with_capacity(right_keys.len());
    for (index, row) in right_rows.iter().enumerate() {
        insert_build_row(
            &mut hash,
            row,
            &right_keys,
            &mut build_key,
            index,
            query.any && matches!(query.kind, JoinType::Inner | JoinType::Left),
        );
    }
    let keep = |pair: &Pair| {
        null_filter.is_none_or(|(index, not_null)| {
            let is_null = cell_at(&left_rows, &right_rows, pair, index, width)
                .is_none_or(|cell| matches!(cell, typed::Cell::Null));
            is_null != not_null
        })
    };
    let mut pairs: Vec<Pair> = Vec::new();
    let count_only = query.aggregate == Some(JoinAggregate::CountStar);
    let mut count = 0u64;
    // Reuse JOIN probe storage instead of allocating a fresh key Vec per left row.
    let mut lookup_key = Vec::with_capacity(left_keys.len());
    let mut matched_right = matches!(query.kind, JoinType::Right | JoinType::Full)
        .then(|| vec![false; right_rows.len()]);
    for (left_index, row) in left_rows.iter().enumerate() {
        let mut matched = false;
        if row_key_into(row, &left_keys, &mut lookup_key) {
            if let Some(matches) = hash.lookup(lookup_key.as_slice()) {
                for &right_index in
                    matches
                        .iter()
                        .take(if query.any && query.kind != JoinType::Right {
                            1
                        } else {
                            usize::MAX
                        })
                {
                    if query.any
                        && query.kind == JoinType::Right
                        && matched_right
                            .as_ref()
                            .is_some_and(|flags| flags[right_index])
                    {
                        continue;
                    }
                    matched = true;
                    if let Some(flags) = matched_right.as_mut() {
                        flags[right_index] = true;
                    }
                    let pair = (Some(left_index), Some(right_index));
                    if keep(&pair) {
                        retain_or_count(&mut pairs, &mut count, count_only, pair)?;
                    }
                }
            }
        }
        if !matched && matches!(query.kind, JoinType::Left | JoinType::Full) {
            let pair = (Some(left_index), None);
            if keep(&pair) {
                retain_or_count(&mut pairs, &mut count, count_only, pair)?;
            }
        }
    }
    if let Some(matched_right) = matched_right {
        for (right_index, matched) in matched_right.into_iter().enumerate() {
            if !matched {
                let pair = (None, Some(right_index));
                if keep(&pair) {
                    retain_or_count(&mut pairs, &mut count, count_only, pair)?;
                }
            }
        }
    }
    if count_only {
        writeln!(
            output,
            "{}",
            query.aggregate_alias.as_deref().unwrap_or("count")
        )?;
        if query.offset == 0 {
            writeln!(output, "{count}")?;
        }
        return Ok(());
    }
    if !order.is_empty() {
        let null = typed::Cell::Null;
        pairs.sort_by(|first, second| {
            for &(index, descending, nulls_first) in &order {
                let a = cell_at(&left_rows, &right_rows, first, index, width).unwrap_or(&null);
                let b = cell_at(&left_rows, &right_rows, second, index, width).unwrap_or(&null);
                let comparison = typed::compare_order_cells(a, b, descending, nulls_first);
                if comparison != std::cmp::Ordering::Equal {
                    return comparison;
                }
            }
            std::cmp::Ordering::Equal
        });
    }
    let header = selected_header(&selected);
    writeln!(output, "{header}")?;
    let projection_types = join_output_types(left_path, right_path, &selected)?;
    for pair in pairs
        .iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (position, (index, _, fallback)) in selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            let null = typed::Cell::Null;
            let value = cell_at(&left_rows, &right_rows, pair, *index, width)
                .or_else(|| {
                    fallback.and_then(|index| cell_at(&left_rows, &right_rows, pair, index, width))
                })
                .unwrap_or(&null);
            typed::write_typed_value(output, value, &projection_types[position])?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}
