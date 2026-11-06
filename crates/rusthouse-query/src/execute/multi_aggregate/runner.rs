//! Keep multi-aggregate expression and accumulator helpers separate from scan execution.

use super::*;

pub(crate) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    items: &[AggregateItem],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut names = Vec::new();
    for item in items {
        if let Some(expression) = &item.argument {
            expression.referenced_columns(&mut names);
        }
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let name = predicate.column();
        if !names.iter().any(|column| column == name) {
            names.push(name.to_owned());
        }
        if let Some(other) = predicate.other_column() {
            if !names.iter().any(|column| column == other) {
                names.push(other.to_owned());
            }
        }
    }
    if names.is_empty() {
        names.push(
            metadata
                .columns
                .first()
                .ok_or_else(|| ExecutionError::Invalid("table has no columns".into()))?
                .name
                .clone(),
        );
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
    for name in &names {
        if !types.contains_key(name) {
            return Err(ExecutionError::Invalid(format!("unknown column: {name}")));
        }
    }
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    writeln!(
        output,
        "{}",
        items
            .iter()
            .map(AggregateItem::header)
            .collect::<Vec<_>>()
            .join("\t")
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }
    // Non-nullable COUNT columns have exactly the snapshot's validated row
    // count. Answer any number of these metrics from metadata, without
    // opening or decompressing a single column stream.
    let metadata_counts = query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && items.iter().all(|item| {
            item.function.is_none()
                && !item.distinct
                && item.argument.as_ref().is_none_or(|expression| {
                    matches!(expression, ScalarExpr::Column(column)
                        if types.get(column).is_some_and(|kind|
                            !kind.starts_with("Nullable(")))
                })
        });
    if metadata_counts {
        if query.offset.unwrap_or(0) == 0 {
            for (index, _) in items.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\t")?;
                }
                write!(output, "{}", metadata.rows)?;
            }
            output.write_all(b"\n")?;
        }
        return Ok(());
    }
    // Mixed nullable COUNTs only need their compact null maps. A full
    // typed value scan decodes every numeric or String payload needlessly.
    // De-duplicate repeated nullable columns across up to 128 metrics.
    let nullmap_counts = query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && items.iter().all(|item| {
            item.function.is_none()
                && !item.distinct
                && item.argument.as_ref().is_none_or(|expression| {
                    matches!(expression, ScalarExpr::Column(column)
                        if types.contains_key(column))
                })
        });
    if nullmap_counts {
        let mut cached = HashMap::<String, u64>::new();
        for item in items {
            let Some(ScalarExpr::Column(column)) = &item.argument else {
                continue;
            };
            if !types[column].starts_with("Nullable(") || cached.contains_key(column) {
                continue;
            }
            let mut reader = TypedQueryReader::open_with_metadata(
                part,
                std::slice::from_ref(column),
                &metadata,
            )?;
            let mut nonnull = 0u64;
            while let Some(nulls) = reader.next_null_batch()? {
                let count = nulls.iter().filter(|&&value| value == 0).count() as u64;
                nonnull = nonnull
                    .checked_add(count)
                    .ok_or_else(|| ExecutionError::Invalid("COUNT overflow".into()))?;
            }
            cached.insert(column.clone(), nonnull);
        }
        if query.offset.unwrap_or(0) == 0 {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\t")?;
                }
                let count = match &item.argument {
                    Some(ScalarExpr::Column(column)) if types[column].starts_with("Nullable(") => {
                        cached[column]
                    }
                    _ => metadata.rows,
                };
                write!(output, "{count}")?;
            }
            output.write_all(b"\n")?;
        }
        return Ok(());
    }
    let mut states = items
        .iter()
        .map(|item| match item.function {
            Some(function) => State::Numeric(AggregateState::new(function)),
            None if item.distinct => {
                let bitmap32 = match item.argument.as_ref() {
                    Some(ScalarExpr::Column(column))
                        if types.get(column).is_some_and(|kind| {
                            matches!(typed::base_type(kind), "UInt32" | "Int32")
                        }) =>
                    {
                        Some(BitmapDistinct32::new())
                    }
                    _ => None,
                };
                State::Distinct(DistinctValues {
                    bitmap32,
                    ..DistinctValues::default()
                })
            }
            None => State::Count(0),
        })
        .collect::<Vec<_>>();
    let arguments = items
        .iter()
        .map(|item| {
            item.argument
                .as_ref()
                .map(|expression| BoundExpr::bind(expression, &indices))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let parallel_safe =
        items.iter().all(
            |item| match (&item.function, &item.argument, item.distinct) {
                (None, _, false) => true,
                (
                    Some(
                        crate::AggregateFunction::Sum
                        | crate::AggregateFunction::Min
                        | crate::AggregateFunction::Max,
                    ),
                    Some(crate::ScalarExpr::Column(column)),
                    false,
                ) => types.get(column).is_some_and(|kind| {
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
                }),
                _ => false,
            },
        );
    // Several affine SUMs over one narrow integer column share one scan and
    // differ only by a constant multiplied by the non-NULL row count.
    let affine = if no_filters {
        affine_sum_states(part, &metadata, &names, &types, items, &arguments)?
    } else {
        None
    };
    let parallel = if affine.is_some() {
        affine
    } else if parallel_safe {
        parallel_exact_multi_aggregates(part, &names, items, &arguments, &conditions, no_filters)?
    } else {
        None
    };
    let scan_serially = parallel.is_none();
    if let Some(partial) = parallel {
        states = partial;
    }
    let mut reader = if scan_serially {
        Some(TypedQueryReader::open_with_metadata(
            part, &names, &metadata,
        )?)
    } else {
        None
    };
    while let Some(batch) = reader
        .as_mut()
        .map_or(Ok(None), TypedQueryReader::next_batch)?
    {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            for (state, argument) in states.iter_mut().zip(&arguments) {
                match state {
                    State::Count(value) => {
                        if count_argument(argument.as_ref(), &batch, row)? {
                            *value = value
                                .checked_add(1)
                                .ok_or_else(|| ExecutionError::Invalid("COUNT overflow".into()))?;
                        }
                    }
                    State::Distinct(seen) => {
                        let expression = argument.as_ref().ok_or_else(|| {
                            ExecutionError::Invalid("distinct argument missing".into())
                        })?;
                        if let BoundExpr::Column(index) = expression {
                            if seen.insert_column(&batch[*index], row)? {
                                continue;
                            }
                        }
                        if let Some(key) = distinct_argument(expression, &batch, row)? {
                            seen.insert_key(key)?;
                        }
                    }
                    State::Numeric(state) => {
                        let expression = argument.as_ref().ok_or_else(|| {
                            ExecutionError::Invalid("aggregate argument missing".into())
                        })?;
                        if let Some(value) = eval_number(expression, &batch, row)? {
                            state.update(value)?;
                        }
                    }
                }
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        for (index, state) in states.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\t")?;
            }
            match state {
                State::Count(count) => write!(output, "{count}")?,
                State::Distinct(seen) => write!(output, "{}", seen.len())?,
                State::Numeric(state) => {
                    let mut value = state.render();
                    if matches!(
                        items[index].function,
                        Some(crate::AggregateFunction::Min | crate::AggregateFunction::Max)
                    ) {
                        if let Some(ScalarExpr::Column(column)) = &items[index].argument {
                            if let Some(data_type) = types.get(column) {
                                value = typed::format_temporal_aggregate(value, data_type);
                            }
                        }
                    }
                    write!(output, "{value}")?;
                }
            }
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}
