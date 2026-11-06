//! Keep JOIN binding, column planning, and row execution in distinct modules.

use super::*;

pub(super) fn execute_multi_join_inner<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    exists_plan: Option<(&[crate::JoinExistsFilter], &[Vec<usize>], &[PathBuf])>,
    avg_plan: Option<(&crate::JoinCorrelatedAvg, &[PathBuf])>,
    min_plan: Option<(&crate::JoinCorrelatedMin, &[PathBuf])>,
    sum_plan: Option<(&crate::JoinCorrelatedSum, &std::path::Path)>,
    stream_buffered_sum: bool,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if query
        .grouped_formula
        .as_ref()
        .is_some_and(|formula| formula.sums.is_empty() || formula.sums.len() > 8)
    {
        return Err(invalid("grouped formula requires between 1 and 8 sums"));
    }
    if query.grouped_formula.is_some()
        && (query.grouped_count.is_none()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
            || query.grouped_sum.is_some()
            || query.grouped_min.is_some()
            || query.grouped_distinct.is_some()
            || query.grouped_count_column.is_some()
            || query.aggregate.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
            || !query.grouped_case_counts.is_empty()
            || query.grouped_sum_having.is_some())
    {
        return Err(invalid("grouped formula requires an INNER grouped plan"));
    }
    if query.grouped_min.is_some()
        && (query.grouped_count.is_none()
            || query.grouped_sum.is_some()
            || query.grouped_distinct.is_some()
            || query.grouped_count_column.is_some()
            || query.aggregate.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
            || !query.grouped_case_counts.is_empty()
            || query.grouped_sum_having.is_some()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner))
    {
        return Err(invalid("grouped MIN requires an INNER grouped plan"));
    }
    if query.on_right_like.is_some() {
        return Err(invalid(
            "JOIN ON LIKE requires the two-table LEFT JOIN executor",
        ));
    }
    if query.grouped_count_column.is_some() {
        return Err(invalid(
            "grouped COUNT(column) requires the two-table JOIN executor",
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
    if query.grouped_distinct.is_some()
        && (query.grouped_count.is_none()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
            || query.grouped_sum.is_some()
            || query.aggregate.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
            || !query.grouped_case_counts.is_empty()
            || !query.where_in_filters.is_empty()
            || !query.where_like_filters.is_empty())
    {
        return Err(invalid(
            "grouped JOIN COUNT DISTINCT requires an INNER grouped plan",
        ));
    }
    if paths.len() != query.steps.len() + 2 {
        return Err(invalid("JOIN source count differs from its plan"));
    }
    let mut relations = Vec::with_capacity(paths.len());
    let mut width = 0usize;
    for (index, path) in paths.iter().enumerate() {
        let metadata = inspect_query_source(path)?;
        let mut names = metadata
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let mut types = metadata
            .columns
            .iter()
            .map(|column| column.data_type.clone())
            .collect::<Vec<_>>();
        let alias = match index {
            0 => query.left_alias.clone(),
            1 => query.right_alias.clone(),
            _ => query.steps[index - 2].alias.clone(),
        };
        let physical_columns = names.len();
        let mut virtual_columns = Vec::new();
        for virtual_column in query
            .virtual_columns
            .iter()
            .filter(|column| column.relation == alias)
        {
            if names.iter().any(|name| name == &virtual_column.name) {
                return Err(invalid(
                    "derived JOIN column conflicts with a stored column",
                ));
            }
            let source_index = names
                .iter()
                .position(|name| name == &virtual_column.source)
                .ok_or_else(|| invalid("derived JOIN source column does not exist"))?;
            let virtual_index = names.len();
            names.push(virtual_column.name.clone());
            types.push("String".into());
            virtual_columns.push(BoundVirtualColumn {
                index: virtual_index,
                source_index,
                start: virtual_column.start,
                length: virtual_column.length,
            });
        }
        let start = width;
        width = width
            .checked_add(names.len())
            .ok_or_else(|| invalid("JOIN column count overflow"))?;
        if width > MAX_CHAIN_COLUMNS {
            return Err(invalid("JOIN supports at most 256 source columns"));
        }
        relations.push(Relation {
            alias,
            names,
            types,
            physical_columns,
            virtual_columns,
            start,
            rows: metadata.rows,
        });
    }
    // EXISTS columns are rebound by relation alias after reordering.
    if avg_plan.is_none() && min_plan.is_none() && sum_plan.is_none() {
        let sizes = relations
            .iter()
            .map(|relation| relation.rows)
            .collect::<Vec<_>>();
        if let Some((reordered, order)) = reorder::plan(query, &sizes) {
            let reordered_paths = order
                .into_iter()
                .map(|index| paths[index].clone())
                .collect::<Vec<_>>();
            return execute_multi_join_inner(
                &reordered,
                &reordered_paths,
                exists_plan,
                None,
                None,
                None,
                stream_buffered_sum,
                output,
            );
        }
    }
    let exists = if let Some((filters, _, inner_paths)) = exists_plan {
        if (query.grouped_count.is_none() && query.scalar_aggregates.is_empty())
            || query.aggregate.is_some()
            || query.grouped_sum.is_some()
            || query.grouped_distinct.is_some()
            || query.grouped_count_column.is_some()
            || !query.grouped_case_counts.is_empty()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        {
            return Err(invalid(
                "correlated JOIN EXISTS requires an INNER aggregate",
            ));
        }
        exists::bind(filters, inner_paths, &relations)?
    } else {
        Vec::new()
    };
    let exists_expression = exists_plan.map_or(&[][..], |(_, expression, _)| expression);
    let avg = if let Some((plan, inner_paths)) = avg_plan {
        if (query.scalar_formula.is_none() && query.scalar_sum.is_none() && query.columns.is_none())
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        {
            return Err(invalid(
                "correlated AVG requires a scalar INNER JOIN aggregate",
            ));
        }
        Some(correlated_avg::bind(plan, inner_paths, &relations)?)
    } else {
        None
    };

    if !query.grouped_key_aliases.is_empty() {
        let simple_projection = query.grouped_count.is_none()
            && !query.grouped_keys_only
            && query.aggregate.is_none()
            && query.scalar_aggregates.is_empty()
            && query.scalar_sum.is_none()
            && query.scalar_formula.is_none()
            && query.grouped_sum.is_none()
            && query.grouped_min.is_none()
            && query.grouped_formula.is_none()
            && query.grouped_window_ratio.is_none();
        let simple_projection_outer_chain = simple_projection
            && matches!(query.kind, JoinType::Inner | JoinType::Left)
            && query
                .steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left));
        let grouped_sum_outer_chain = query.grouped_sum.is_some()
            && query.grouped_count.is_some()
            && query
                .columns
                .as_ref()
                .is_some_and(|columns| !columns.is_empty())
            && matches!(query.kind, JoinType::Inner | JoinType::Left)
            && query
                .steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left));
        if !(query.grouped_count.is_some() || simple_projection)
            || (!(simple_projection_outer_chain || grouped_sum_outer_chain)
                && (query.kind != JoinType::Inner
                    || query.steps.iter().any(|step| step.kind != JoinType::Inner)))
        {
            return Err(invalid(
                "JOIN key aliases require a simple or grouped INNER join",
            ));
        }
        for (position, alias) in &query.grouped_key_aliases {
            if query
                .columns
                .as_ref()
                .is_none_or(|columns| *position >= columns.len())
                || (query.grouped_count.is_some()
                    && relations.iter().any(|relation| {
                        relation
                            .names
                            .iter()
                            .any(|name| name.eq_ignore_ascii_case(alias))
                    }))
            {
                return Err(invalid("JOIN key alias conflicts with a source column"));
            }
        }
    }

    if let Some((position, alias)) = &query.grouped_year {
        if relations.iter().any(|relation| {
            relation
                .names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(alias))
        }) {
            return Err(invalid("EXTRACT YEAR alias conflicts with a source column"));
        }
        let column = query
            .columns
            .as_ref()
            .and_then(|columns| columns.get(*position))
            .ok_or_else(|| invalid("EXTRACT YEAR requires a projected column"))?;
        if query.grouped_count.is_none()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        {
            return Err(invalid("EXTRACT YEAR requires a grouped INNER join"));
        }
        let kind = typed::base_type(column_type(&relations, column_index(&relations, column)?));
        if !matches!(kind, "Date" | "Date32") {
            return Err(invalid("EXTRACT YEAR requires Date or Date32"));
        }
    }

    let min_filter = if let Some((plan, inner_paths)) = min_plan {
        if query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
            || query.aggregate.is_some()
            || query.grouped_count.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
        {
            return Err(invalid("correlated MIN requires an INNER JOIN projection"));
        }
        Some(correlated_min::bind(
            plan,
            inner_paths,
            &relations,
            query.limit == Some(0),
        )?)
    } else {
        None
    };

    let sum_filter = if let Some((plan, inner)) = sum_plan {
        if query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
            || query.aggregate.is_some()
            || query.grouped_count.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
        {
            return Err(invalid("correlated SUM requires an INNER JOIN projection"));
        }
        Some(correlated_sum::bind(
            plan,
            inner,
            &relations,
            query.limit == Some(0),
        )?)
    } else {
        None
    };

    let initial_step = JoinStep {
        table: query.right.clone(),
        alias: query.right_alias.clone(),
        kind: query.kind,
        any: query.any,
        keys: query.keys.clone(),
        using_keys: query.using_keys.clone(),
    };
    let scalar_metric = match (&query.scalar_sum, &query.scalar_sum_qualifier) {
        (Some(crate::ScalarExpr::Column(name)), Some(relation)) => {
            Some(crate::join::GroupedSum::Column(JoinColumn {
                relation: relation.clone(),
                name: name.clone(),
            }))
        }
        (Some(expression), None) => Some(crate::join::GroupedSum::Expression(expression.clone())),
        (None, None) => None,
        _ => return Err(invalid("invalid qualified scalar SUM")),
    };
    let bind_sum = |metric: &crate::join::GroupedSum| -> Result<BoundSum, ExecutionError> {
        let (expression, names) = match metric {
            crate::join::GroupedSum::Column(column) => {
                let index = column_index(&relations, column)?;
                (
                    super::super::multi_aggregate::BoundExpr::Column(index),
                    vec![index],
                )
            }
            crate::join::GroupedSum::Expression(expression) => {
                let mut columns = Vec::new();
                expression.referenced_columns(&mut columns);
                let mut indices = HashMap::new();
                let mut names = Vec::new();
                for name in columns {
                    let (relation, column_name) = name.split_once('\0').unwrap_or(("", &name));
                    let column = JoinColumn {
                        relation: relation.to_owned(),
                        name: column_name.to_owned(),
                    };
                    let index = column_index(&relations, &column)?;
                    indices.insert(name, index);
                    names.push(index);
                }
                (
                    super::super::multi_aggregate::BoundExpr::bind(expression, &indices)?,
                    names,
                )
            }
        };
        validate_sum_expression(&expression, &relations)?;
        Ok(BoundSum {
            expression,
            indices: names,
        })
    };
    let min_metric = query
        .grouped_min
        .as_ref()
        .map(|column| crate::join::GroupedSum::Column(column.clone()));
    let grouped_sum = query
        .grouped_sum
        .as_ref()
        .or(scalar_metric.as_ref())
        .or(min_metric.as_ref())
        .map(bind_sum)
        .transpose()?;
    let grouped_sum_extras = query
        .grouped_sum_extras
        .iter()
        .map(|(metric, _)| bind_sum(metric))
        .collect::<Result<Vec<_>, _>>()?;
    let formula = query
        .scalar_formula
        .as_ref()
        .or(query.grouped_formula.as_ref())
        .map(|formula| -> Result<BoundFormula, ExecutionError> {
            let sums = formula
                .sums
                .iter()
                .map(|expression| {
                    bind_sum(&crate::join::GroupedSum::Expression(expression.clone()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let indices = (0..sums.len())
                .map(|index| (format!("__join_sum_{index}"), index))
                .collect::<HashMap<_, _>>();
            let expression =
                super::super::multi_aggregate::BoundExpr::bind(&formula.expression, &indices)?;
            Ok(BoundFormula { sums, expression })
        })
        .transpose()?;
    let distinct_index = query
        .grouped_distinct
        .as_ref()
        .map(|column| {
            let index = column_index(&relations, column)?;
            let kind = typed::base_type(column_type(&relations, index));
            if !matches!(
                kind,
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
            ) {
                return Err(invalid(
                    "grouped JOIN COUNT DISTINCT requires an integer column",
                ));
            }
            Ok(index)
        })
        .transpose()?;
    let count_column = if let Some(crate::JoinAggregate::CountColumn(column)) = &query.aggregate {
        let index = column_index(&relations, column)?;
        let header = query
            .aggregate_alias
            .clone()
            .unwrap_or_else(|| format!("count({})", column.display_name()));
        Some((index, header))
    } else {
        None
    };
    // Bind predicates before early returns, including LIMIT 0.
    let comparisons = query
        .where_comparison
        .iter()
        .chain(&query.where_comparisons_extra)
        .map(|(column, operator, literal)| {
            let index = column_index(&relations, column)?;
            let value = typed::parse_literal(column_type(&relations, index), literal)?;
            Ok((index, *operator, value))
        })
        .collect::<Result<Vec<_>, ExecutionError>>()?;

    let column_comparisons = query
        .where_column_comparisons
        .iter()
        .map(|(left, operator, right)| {
            let left = column_index(&relations, left)?;
            let right = column_index(&relations, right)?;
            let left_type = typed::base_type(column_type(&relations, left));
            let right_type = typed::base_type(column_type(&relations, right));
            let numeric = |kind: &str| {
                matches!(
                    kind,
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
            };
            if left_type != right_type && !(numeric(left_type) && numeric(right_type)) {
                return Err(invalid("JOIN column comparison requires compatible types"));
            }
            Ok((left, *operator, right))
        })
        .collect::<Result<Vec<_>, ExecutionError>>()?;

    // Bind both source columns and require textual types before early returns.
    let substring_comparisons = query
        .where_substring_comparisons
        .iter()
        .map(
            |(left, start, length, operator, right, right_start, right_length)| {
                let left_index = column_index(&relations, left)?;
                let right_index = column_index(&relations, right)?;
                if typed::base_type(column_type(&relations, left_index)) != "String"
                    || typed::base_type(column_type(&relations, right_index)) != "String"
                {
                    return Err(invalid("SUBSTRING JOIN comparisons require String columns"));
                }
                Ok((
                    left_index,
                    *start,
                    *length,
                    *operator,
                    right_index,
                    *right_start,
                    *right_length,
                ))
            },
        )
        .collect::<Result<Vec<BoundSubstringComparison>, ExecutionError>>()?;

    // Two-table parsers may store IN and LIKE separately from boolean branches.
    // Correlated plans also use this executor, so retain those conjuncts here.
    let row_filters = if query.scalar_where.is_some()
        || !query.where_in_filters.is_empty()
        || !query.where_like_filters.is_empty()
    {
        let mut branches = query.scalar_where.clone().unwrap_or_else(|| vec![vec![]]);
        let column_name = |column: &JoinColumn| {
            if column.relation.is_empty() {
                column.name.clone()
            } else {
                format!("{}\0{}", column.relation, column.name)
            }
        };
        for branch in &mut branches {
            branch.extend(query.where_in_filters.iter().map(|(column, values)| {
                crate::Predicate::In {
                    column: column_name(column),
                    values: values.clone(),
                }
            }));
            branch.extend(query.where_like_filters.iter().map(
                |(column, pattern, case_insensitive, negated)| crate::Predicate::Like {
                    column: column_name(column),
                    pattern: pattern.clone(),
                    case_insensitive: *case_insensitive,
                    negated: *negated,
                },
            ));
        }
        Some(row_filters::RowFilters::bind(&branches, &relations)?)
    } else {
        None
    };

    // Validate the complete output schema without materializing rows.
    if query.limit == Some(0)
        || ((query.aggregate.is_some()
            || query.scalar_sum.is_some()
            || !query.scalar_aggregates.is_empty()
            || query.scalar_formula.is_some())
            && query.offset > 0)
    {
        for (index, join) in std::iter::once(&initial_step)
            .chain(query.steps.iter())
            .enumerate()
        {
            let width = relations[index + 1].start;
            step_keys(join, &relations[..=index + 1], width)?;
        }
        if let Some((column, _)) = &query.where_null {
            column_index(&relations, column)?;
        }
        for (column, _, _) in &query.order_by {
            if !(column.is_count_order()
                && (query.grouped_count.is_some()
                    || query.aggregate == Some(crate::JoinAggregate::CountStar)
                    || !query.scalar_aggregates.is_empty()
                    || query.scalar_sum.is_some()))
                && column.scalar_order_index().is_none()
            {
                column_index(&relations, column)?;
            }
        }
        if !query.scalar_aggregates.is_empty() {
            write_scalar_aggregate_header(query, output)?;
        } else if query.scalar_sum.is_some() || query.scalar_formula.is_some() {
            write_scalar_sum(query, None, false, output)?;
        } else if let Some((_, header)) = &count_column {
            writeln!(output, "{header}")?;
        } else if query.aggregate == Some(crate::JoinAggregate::CountStar) {
            writeln!(
                output,
                "{}",
                query.aggregate_alias.as_deref().unwrap_or("count_star()")
            )?;
        } else {
            let selected = output_columns(query, &relations, &initial_step)?;
            if query.grouped_count.is_some() {
                write_group_header(query, &selected, output)?;
            } else {
                write_join_header(output, &selected)?;
            }
        }
        return Ok(());
    }

    // A left-deep INNER chain can stream its large fact side when each
    // right-hand build table fits the shared chain memory budget. Grouped
    // CASE metrics also support LEFT stages for NULL checks.
    let streamable_join_chain = (query.kind == JoinType::Inner
        && query.steps.iter().all(|step| step.kind == JoinType::Inner))
        || (!query.grouped_case_counts.is_empty()
            && matches!(query.kind, JoinType::Inner | JoinType::Left)
            && query
                .steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left)))
        || (query.grouped_sum.is_some()
            && query.grouped_count.is_some()
            && matches!(query.kind, JoinType::Inner | JoinType::Left)
            && query
                .steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left)));
    if streamable_join_chain
        && (query.order_by.is_empty()
            || query.grouped_count.is_some()
            || (!query.scalar_projections.is_empty() && query.grouped_count.is_none())
            || (!query.scalar_aggregates.is_empty()
                && query
                    .order_by
                    .iter()
                    .all(|(column, _, _)| column.is_count_order()))
            || (query.scalar_sum.is_some()
                && query
                    .order_by
                    .iter()
                    .all(|(column, _, _)| column.is_count_order())))
        && query.aggregate.is_none()
        && min_filter.is_none()
        && (sum_filter.is_none() || stream_buffered_sum)
    {
        return streaming::execute(
            query,
            paths,
            &relations,
            &initial_step,
            &comparisons,
            &column_comparisons,
            &substring_comparisons,
            grouped_sum.as_ref(),
            &grouped_sum_extras,
            formula.as_ref(),
            distinct_index,
            row_filters.as_ref(),
            &exists,
            exists_expression,
            avg.as_ref(),
            sum_filter.as_ref(),
            output,
        );
    }

    // Resolve the WHERE column across all relations before materialization.
    // Filtering early is safe only at the final JOIN, after null extension.
    let output_filter = query
        .where_null
        .as_ref()
        .map(|(column, not_null)| column_index(&relations, column).map(|index| (index, *not_null)))
        .transpose()?;
    let required = required_physical_columns(
        query,
        &relations,
        &initial_step,
        row_filters.as_ref(),
        avg.as_ref(),
        min_filter.as_ref(),
        sum_filter.as_ref(),
    )?;
    let min_filter_relation = min_filter
        .as_ref()
        .map(|filter| {
            relations
                .iter()
                .position(|relation| {
                    filter.required_row_width() <= relation.start + relation.names.len()
                })
                .ok_or_else(|| invalid("correlated MIN columns exceed the JOIN row width"))
        })
        .transpose()?;
    // Correlated MIN needs a materialized ordered result. Its projected inputs
    // can use the same bounded build-row cap as streaming joins; retained
    // bytes and intermediate matches are still checked independently.
    let input_row_limit = if min_filter.is_some()
        || (query.aggregate == Some(crate::JoinAggregate::CountStar)
            && query.kind == JoinType::Inner
            && query.steps.iter().all(|step| step.kind == JoinType::Inner))
    {
        crate::join::MAX_STREAM_JOIN_BUILD_ROWS
    } else {
        MAX_INPUT_ROWS
    };
    let inner_join_plan = query.kind == JoinType::Inner
        && query.steps.iter().all(|step| step.kind == JoinType::Inner);
    let first_relation_filters = if inner_join_plan {
        relation_comparison_filters(&comparisons, 0, relations[0].physical_columns)
    } else {
        Vec::new()
    };
    let first_row_filter = row_filters
        .as_ref()
        .filter(|filters| {
            inner_join_plan && filters.can_prefilter_relation(0, relations[0].names.len())
        })
        .map(|filters| {
            let conditions = filters.prefilter_conditions(0, relations[0].names.len());
            (filters, conditions)
        });
    let mut rows = if first_relation_filters.is_empty() && first_row_filter.is_none() {
        read_projected_build_rows(
            &paths[0],
            &relations[0].names,
            &required[0],
            relations[0].rows,
            input_row_limit,
            MAX_STREAM_BUILD_BYTES,
        )?
    } else {
        let mut predicate = |row: &[typed::Cell]| {
            passes_relation_comparison_filters(row, &first_relation_filters)
                && first_row_filter
                    .as_ref()
                    .map_or(true, |(filters, conditions)| {
                        filters.passes_relation(row, 0, conditions)
                    })
        };
        read_projected_build_rows_matching(
            &paths[0],
            &relations[0].names,
            &required[0],
            relations[0].rows,
            input_row_limit,
            MAX_STREAM_BUILD_BYTES,
            &mut predicate,
        )?
    };
    if min_filter_relation == Some(0) {
        let filter = min_filter
            .as_ref()
            .ok_or_else(|| invalid("correlated MIN relation has no filter"))?;
        let mut filtered = Vec::with_capacity(rows.len());
        for row in rows {
            if filter.passes(&row)? {
                filtered.push(row);
            }
        }
        rows = filtered;
    }
    for (step_index, step) in std::iter::once(&initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let right_relation = &relations[step_index + 1];
        let left_width = right_relation.start;
        let (left_keys, right_keys) = step_keys(step, &relations[..=step_index + 1], left_width)?;
        let apply_min_filter = min_filter_relation == Some(step_index + 1);
        // INNER and LEFT cannot create rows from an empty intermediate.
        // Keep the source row limit even when its payload is not needed.
        if rows.is_empty() && matches!(step.kind, JoinType::Inner | JoinType::Left) {
            if right_relation.rows > input_row_limit {
                return Err(invalid(format!(
                    "JOIN supports at most {input_row_limit} input rows per table"
                )));
            }
            continue;
        }
        let right_filters = if inner_join_plan {
            relation_comparison_filters(
                &comparisons,
                right_relation.start,
                right_relation.physical_columns,
            )
        } else {
            Vec::new()
        };
        let right_row_filter = row_filters
            .as_ref()
            .filter(|filters| {
                inner_join_plan
                    && filters
                        .can_prefilter_relation(right_relation.start, right_relation.names.len())
            })
            .map(|filters| {
                let conditions =
                    filters.prefilter_conditions(right_relation.start, right_relation.names.len());
                (filters, conditions)
            });
        let right_rows = if right_filters.is_empty() && right_row_filter.is_none() {
            read_projected_build_rows(
                &paths[step_index + 1],
                &right_relation.names,
                &required[step_index + 1],
                right_relation.rows,
                input_row_limit,
                MAX_STREAM_BUILD_BYTES,
            )?
        } else {
            let mut predicate = |row: &[typed::Cell]| {
                passes_relation_comparison_filters(row, &right_filters)
                    && right_row_filter
                        .as_ref()
                        .map_or(true, |(filters, conditions)| {
                            filters.passes_relation(row, right_relation.start, conditions)
                        })
            };
            read_projected_build_rows_matching(
                &paths[step_index + 1],
                &right_relation.names,
                &required[step_index + 1],
                right_relation.rows,
                input_row_limit,
                MAX_STREAM_BUILD_BYTES,
                &mut predicate,
            )?
        };
        let mut hash: HashMap<Vec<typed::Key>, Bucket> = HashMap::new();
        let mut hash_bytes = 0usize;
        let mut build_key = Vec::with_capacity(right_keys.len());
        for (index, row) in right_rows.iter().enumerate() {
            if !row_key_into(row, &right_keys, &mut build_key) {
                continue;
            }
            // Lookup does not own the key. Skewed dimensions allocate only
            // for distinct hash entries instead of every duplicate row.
            if let Some(bucket) = hash.get_mut(build_key.as_slice()) {
                charge_hash_bytes(&mut hash_bytes, std::mem::size_of::<usize>() * 2)?;
                bucket.push(index);
            } else {
                charge_hash_bytes(&mut hash_bytes, hash_key_bytes(&build_key)?)?;
                hash.insert(build_key.clone(), Bucket::One(index));
            }
        }
        // COUNT(*) over a validated all-INNER chain needs only the final
        // hash-bucket multiplicities. No final joined rows are retained.
        if query.aggregate == Some(crate::JoinAggregate::CountStar)
            && query.kind == JoinType::Inner
            && query.steps.iter().all(|step| step.kind == JoinType::Inner)
            && avg.is_none()
            && step_index == query.steps.len()
            && comparisons.is_empty()
            && column_comparisons.is_empty()
            && row_filters.is_none()
        {
            let mut count = 0u64;
            let mut count_key = Vec::with_capacity(left_keys.len());
            for row in &rows {
                if row_key_into(row, &left_keys, &mut count_key) {
                    if let Some(matches) = hash.get(count_key.as_slice()) {
                        // The final COUNT fast path must apply the same
                        // IS NULL / IS NOT NULL filter as materialized joins.
                        // With ANY, inspect only the first matched dimension
                        // row: filtering all matches changes first-match rules.
                        let multiplicity = match output_filter {
                            Some((filter_index, not_null)) if filter_index < left_width => {
                                if matches!(row[filter_index], typed::Cell::Null) == not_null {
                                    0
                                } else if step.any {
                                    1
                                } else {
                                    matches.iter().len()
                                }
                            }
                            Some((filter_index, not_null)) => matches
                                .iter()
                                .take(if step.any { 1 } else { usize::MAX })
                                .filter(|&&right_index| {
                                    matches!(
                                        right_rows[right_index][filter_index - left_width],
                                        typed::Cell::Null
                                    ) != not_null
                                })
                                .count(),
                            None => {
                                if step.any {
                                    1
                                } else {
                                    matches.iter().len()
                                }
                            }
                        };
                        count = count
                            .checked_add(
                                u64::try_from(multiplicity)
                                    .map_err(|_| invalid("JOIN count overflow"))?,
                            )
                            .ok_or_else(|| invalid("JOIN count overflow"))?;
                    }
                }
            }
            writeln!(
                output,
                "{}\n{count}",
                query.aggregate_alias.as_deref().unwrap_or("count_star()")
            )?;
            return Ok(());
        }
        // Most fact-to-dimension joins preserve the incoming row count.
        let mut next = Vec::with_capacity(rows.len().min(MAX_RESULT_ROWS));
        let mut retained = 0usize;
        let mut matched_right = matches!(step.kind, JoinType::Right | JoinType::Full)
            .then(|| vec![false; right_rows.len()]);
        let filter = if step_index == query.steps.len() {
            output_filter
        } else {
            None
        };
        let mut probe_key = Vec::with_capacity(left_keys.len());
        for row in &rows {
            let mut matched = false;
            if row_key_into(row, &left_keys, &mut probe_key) {
                if let Some(matches) = hash.get(probe_key.as_slice()) {
                    for &match_index in
                        matches
                            .iter()
                            .take(if step.any && step.kind != JoinType::Right {
                                1
                            } else {
                                usize::MAX
                            })
                    {
                        if step.any
                            && step.kind == JoinType::Right
                            && matched_right
                                .as_ref()
                                .is_some_and(|flags| flags[match_index])
                        {
                            continue;
                        }
                        matched = true;
                        if let Some(flags) = matched_right.as_mut() {
                            flags[match_index] = true;
                        }
                        let mut joined =
                            Vec::with_capacity(left_width + right_relation.names.len());
                        joined.extend_from_slice(row);
                        joined.extend_from_slice(&right_rows[match_index]);
                        if apply_min_filter {
                            let filter = min_filter
                                .as_ref()
                                .ok_or_else(|| invalid("correlated MIN relation has no filter"))?;
                            if !filter.passes(&joined)? {
                                continue;
                            }
                        }
                        if step_index != query.steps.len()
                            || (comparisons.iter().all(|(index, operator, literal)| {
                                typed::matches_comparison(&joined[*index], literal, *operator)
                            }) && column_comparisons.iter().all(|(left, operator, right)| {
                                typed::matches_comparison(
                                    &joined[*left],
                                    &joined[*right],
                                    *operator,
                                )
                            }) && match &row_filters {
                                Some(filters) => filters.passes(&joined)?,
                                None => true,
                            } && match &sum_filter {
                                Some(filter) => filter.passes(&joined)?,
                                None => true,
                            }) && match &avg {
                                Some(filter) => filter.passes(&joined)?,
                                None => true,
                            }
                        {
                            push_visible_row(&mut next, joined, &mut retained, filter)?;
                        }
                    }
                }
            }
            if !matched && matches!(step.kind, JoinType::Left | JoinType::Full) {
                let mut joined = Vec::with_capacity(left_width + right_relation.names.len());
                joined.extend_from_slice(row);
                joined.resize(left_width + right_relation.names.len(), typed::Cell::Null);
                let passes_filter = step_index != query.steps.len()
                    || match &row_filters {
                        Some(filters) => filters.passes(&joined)?,
                        None => true,
                    };
                if passes_filter {
                    push_visible_row(&mut next, joined, &mut retained, filter)?;
                }
            }
        }
        if let Some(matched_right) = matched_right {
            for (right_index, matched) in matched_right.into_iter().enumerate() {
                if !matched {
                    let mut joined = vec![typed::Cell::Null; left_width];
                    joined.extend_from_slice(&right_rows[right_index]);
                    let passes_filter = step_index != query.steps.len()
                        || match &row_filters {
                            Some(filters) => filters.passes(&joined)?,
                            None => true,
                        };
                    if passes_filter {
                        push_visible_row(&mut next, joined, &mut retained, filter)?;
                    }
                }
            }
        }
        rows = next;
    }

    if !query.order_by.is_empty() && query.aggregate != Some(crate::JoinAggregate::CountStar) {
        let scalar_projections = bind_scalar_projections(query, &relations)?;
        let sort_base = relations
            .iter()
            .map(|relation| relation.names.len())
            .sum::<usize>();
        let ordering_columns = query
            .order_by
            .iter()
            .map(|(column, descending, nulls_first)| {
                let index = column
                    .scalar_order_index()
                    .map(|index| sort_base + index)
                    .map_or_else(|| column_index(&relations, column), Ok)?;
                Ok((index, *descending, *nulls_first))
            })
            .collect::<Result<Vec<_>, ExecutionError>>()?;
        if query
            .order_by
            .iter()
            .any(|(column, _, _)| column.scalar_order_index().is_some())
        {
            for row in &mut rows {
                let values = scalar_projections
                    .iter()
                    .map(|expression| super::super::multi_aggregate::eval_row_value(expression, row))
                    .collect::<Result<Vec<_>, _>>()?;
                row.extend(values);
            }
        }
        rows.sort_by(|left, right| {
            for (index, descending, nulls_first) in &ordering_columns {
                let ordering = typed::compare_order_cells(
                    &left[*index],
                    &right[*index],
                    *descending,
                    *nulls_first,
                );
                if !ordering.is_eq() {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        });
    }
    if write_global_case_aggregates(query, &rows, &relations, output)? {
        return Ok(());
    }
    if query.aggregate == Some(crate::JoinAggregate::CountStar) {
        writeln!(
            output,
            "{}",
            query.aggregate_alias.as_deref().unwrap_or("count_star()")
        )?;
        if query.offset == 0 {
            writeln!(output, "{}", rows.len())?;
        }
        return Ok(());
    }
    if let Some((index, header)) = count_column {
        let count = rows
            .iter()
            .filter(|row| !matches!(row[index], typed::Cell::Null))
            .count();
        writeln!(output, "{header}\n{count}")?;
        return Ok(());
    }
    let selected = output_columns(query, &relations, &initial_step)?;
    write_join_header(output, &selected)?;
    let scalar_projections = bind_scalar_projections(query, &relations)?;
    for row in rows
        .iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (position, (index, _, fallback)) in selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            if let Some(expression) = scalar_output_index(*index, scalar_projections.len()) {
                let value =
                    super::super::multi_aggregate::eval_row_value(&scalar_projections[expression], row)?;
                typed::write_value(output, &value)?;
                continue;
            }
            let source = if matches!(row[*index], typed::Cell::Null) {
                fallback.unwrap_or(*index)
            } else {
                *index
            };
            typed::write_typed_value(output, &row[source], column_type(&relations, source))?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}
