use super::*;

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
    let mut reader = NumericQueryReader::open(part, &scan_columns)?;
    let mut groups = GroupedCounts::default();
    if query.limit == Some(0) {
        return groups.write(query, column, output);
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
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
            writeln!(output, "{column}\t{}", query.alias.as_deref().unwrap_or("count"))?;
            for index in begin..begin + length {
                let number = u128::from(start) + u128::from(index) * u128::from(step);
                writeln!(output, "{number}\t1")?;
            }
            return Ok(());
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
                    if term.descending { ordering.reverse() } else { ordering }
                };
                if comparison != Ordering::Equal { return comparison; }
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
    let mut scan_columns = vec![column.to_owned()];
    let comparisons = compile_part_where(query, &mut scan_columns, |name, literal| {
        let col = metadata
            .columns
            .iter()
            .find(|candidate| candidate.name == name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown WHERE column: {name}")))?;
        parse_numeric(&col.data_type, literal)
    })?;
    let mut reader = NumericQueryReader::open(part, &scan_columns)?;
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
        for row in 0..batch[0].len() {
            if batch_matches(&batch, row, &comparisons) {
                count = count
                    .checked_add(1)
                    .ok_or_else(|| ExecutionError::Invalid("count overflow".into()))?;
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{count}")?;
    }
    Ok(())
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
    let mut reader = NumericQueryReader::open(part, &scan_columns)?;
    write_header(query, output, &format!("countDistinct({column})"))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let mut seen = HashSet::new();
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !batch_matches(&batch, row, &comparisons) {
                continue;
            }
            let key = NumberKey::from(number_at(&batch[0], row));
            if seen.len() >= MAX_DISTINCT_ROWS && !seen.contains(&key) {
                return Err(ExecutionError::Invalid(
                    "COUNT(DISTINCT) supports at most 100000 unique values".into(),
                ));
            }
            seen.insert(key);
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{}", seen.len())?;
    }
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
    let mut reader = NumericQueryReader::open(part, &scan_columns)?;
    write_header(query, output, &format!("{}({column})", function.name()))?;
    if query.limit == Some(0) {
        return Ok(());
    }
    let mut state = AggregateState::new(function);
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
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
