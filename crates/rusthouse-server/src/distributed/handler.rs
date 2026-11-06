//! Separate distributed request orchestration from shard reducers and merge helpers.

use super::*;

pub fn handle(method: &str, body: &[u8], cluster: &Cluster, max_rows: usize) -> Response {
    if method != "POST" {
        return Response::text(
            "405 Method Not Allowed",
            "distributed query requires POST\n",
        );
    }
    if body.is_empty() || body.len() > MAX_QUERY_BYTES {
        return Response::text("400 Bad Request", "invalid distributed query length\n");
    }
    let sql = match std::str::from_utf8(body) {
        Ok(sql) => sql,
        Err(_) => return Response::text("400 Bad Request", "SQL must be UTF-8\n"),
    };
    let select = match parse_select(sql) {
        Ok(select)
            if is_safe_scan(&select)
                || is_safe_count(&select)
                || is_safe_distinct_scan(&select)
                || is_safe_multi_grouped_distinct(&select)
                || is_safe_grouped_distinct(&select)
                || is_safe_count_distinct(&select)
                || is_safe_uniq(&select)
                || is_safe_sorted_scan(&select)
                || is_safe_count_if(&select)
                || is_safe_grouped_count(&select)
                || is_safe_grouped_multi_count(&select)
                || is_safe_grouped_multi_count_column(&select)
                || is_safe_grouped_multi_bitwise(&select)
                || is_safe_grouped_multi_product(&select)
                || is_safe_grouped_multi_sumsquares(&select)
                || is_safe_grouped_multi_countif(&select)
                || is_safe_grouped_multi_extremum(&select)
                || is_safe_grouped_multi_sum(&select)
                || is_safe_grouped_sum(&select)
                || is_safe_grouped_extremum(&select)
                || is_safe_grouped_product(&select)
                || is_safe_grouped_bitwise(&select)
                || is_safe_sum(&select)
                || is_safe_sum_squares(&select)
                || is_safe_product(&select)
                || is_safe_rms(&select)
                || is_safe_avg(&select)
                || is_safe_range(&select)
                || is_safe_bitwise(&select)
                || is_safe_extremum(&select) =>
        {
            select
        }
        _ => {
            return Response::text(
                "400 Bad Request",
                "distributed queries require unordered SELECT, bounded integer top-K SELECT, COUNT, COUNT(DISTINCT), uniq, COUNTIF, integer SUM, MIN, MAX, or grouped aggregates\n",
            );
        }
    };
    let sorted_query = is_safe_sorted_scan(&select);
    let sorted_prefix =
        sorted_query.then(|| select.offset.unwrap_or(0) + select.limit.unwrap_or(0));
    let count_query = is_safe_count(&select) || is_safe_count_if(&select);
    let distinct_rows_query = is_safe_distinct_scan(&select);
    let multi_grouped_distinct_query = is_safe_multi_grouped_distinct(&select);
    let grouped_distinct_query = is_safe_grouped_distinct(&select);
    let distinct_count_column = match &select.projection {
        Projection::CountDistinct { column } => Some(column.as_str()),
        _ => None,
    };
    let grouped_count_query = is_safe_grouped_count(&select);
    let grouped_multi_count_query = is_safe_grouped_multi_count(&select);
    let grouped_multi_count_column_query = is_safe_grouped_multi_count_column(&select);
    let grouped_multi_bitwise_query = is_safe_grouped_multi_bitwise(&select);
    let grouped_multi_product_query = is_safe_grouped_multi_product(&select);
    let grouped_multi_sumsquares_query = is_safe_grouped_multi_sumsquares(&select);
    let grouped_multi_countif_query = is_safe_grouped_multi_countif(&select);
    let grouped_multi_extremum_query = is_safe_grouped_multi_extremum(&select);
    let grouped_multi_sum_query = is_safe_grouped_multi_sum(&select);
    let grouped_sum_query = is_safe_grouped_sum(&select);
    let grouped_count_column_query = matches!(
        select.projection,
        Projection::GroupedAggregate {
            function: AggregateFunction::Count,
            ..
        }
    );
    let grouped_extremum_query = is_safe_grouped_extremum(&select);
    let grouped_product_query = is_safe_grouped_product(&select);
    let grouped_bitwise_query = is_safe_grouped_bitwise(&select);
    let sum_query = is_safe_sum(&select) || is_safe_sum_squares(&select);
    let product_query = is_safe_product(&select);
    let rms_query = is_safe_rms(&select);
    let avg_query = is_safe_avg(&select);
    let range_query = is_safe_range(&select);
    let uniq_query = is_safe_uniq(&select);
    let shard_sql = if sorted_query {
        match sorted_scan_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported distributed sort\n"),
        }
    } else if multi_grouped_distinct_query {
        match multi_grouped_distinct_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported grouped DISTINCT\n"),
        }
    } else if grouped_distinct_query {
        match grouped_distinct_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported grouped DISTINCT\n"),
        }
    } else if distinct_rows_query {
        match unpaged_scan_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text(
                    "400 Bad Request",
                    "unsupported distributed DISTINCT pagination\n",
                );
            }
        }
    } else if is_safe_scan(&select) {
        match bounded_scan_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text("400 Bad Request", "unsupported distributed pagination\n");
            }
        }
    } else if grouped_sum_query
        || grouped_extremum_query
        || grouped_product_query
        || grouped_bitwise_query
    {
        match grouped_count_shard_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text("400 Bad Request", "unsupported global grouped aggregate\n");
            }
        }
    } else if grouped_count_query {
        match grouped_count_shard_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text("400 Bad Request", "unsupported global grouped query\n");
            }
        }
    } else if rms_query {
        match rms_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported distributed RMS\n"),
        }
    } else if avg_query {
        match avg_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported distributed AVG\n"),
        }
    } else if range_query {
        match range_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported distributed RANGE\n"),
        }
    } else if uniq_query {
        match uniq_state_shard_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text("400 Bad Request", "unsupported distributed uniq query\n");
            }
        }
    } else if distinct_count_column.is_some() {
        match distinct_shard_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text(
                    "400 Bad Request",
                    "unsupported distributed DISTINCT query\n",
                );
            }
        }
    } else {
        sql.to_owned()
    };
    let bitwise_function = match &select.projection {
        Projection::Aggregate {
            function:
                function @ (AggregateFunction::GroupBitAnd
                | AggregateFunction::GroupBitOr
                | AggregateFunction::GroupBitXor),
            ..
        } => Some(*function),
        _ => None,
    };
    let extrema_function = match &select.projection {
        Projection::Aggregate {
            function: function @ (AggregateFunction::Min | AggregateFunction::Max),
            ..
        } => Some(*function),
        _ => None,
    };
    let sum_schema = match (&select.source, &select.projection) {
        (
            Some(Source::Table(table)),
            Projection::Aggregate {
                function:
                    AggregateFunction::Sum
                    | AggregateFunction::SumSquares
                    | AggregateFunction::Rms
                    | AggregateFunction::Avg
                    | AggregateFunction::Range
                    | AggregateFunction::Product
                    | AggregateFunction::Min
                    | AggregateFunction::Max
                    | AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor,
                column,
            },
        ) => Some((table.as_str(), column.as_str())),
        (
            Some(Source::Table(table)),
            Projection::GroupedAggregate {
                function:
                    AggregateFunction::Sum
                    | AggregateFunction::SumSquares
                    | AggregateFunction::Count
                    | AggregateFunction::CountIf
                    | AggregateFunction::Min
                    | AggregateFunction::Max
                    | AggregateFunction::Product
                    | AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor,
                value_column,
                ..
            },
        ) => Some((table.as_str(), value_column.as_str())),
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_count_column_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_bitwise_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_product_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_sumsquares_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_countif_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_extremum_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_sum_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::Columns(columns)) if sorted_query => columns
            .first()
            .map(|column| (table.as_str(), column.as_str())),
        _ => None,
    };
    let group_schema = match (&select.source, &select.projection) {
        (
            Some(Source::Table(table)),
            Projection::GroupedAggregate {
                function:
                    AggregateFunction::Sum
                    | AggregateFunction::SumSquares
                    | AggregateFunction::Count
                    | AggregateFunction::CountIf
                    | AggregateFunction::Min
                    | AggregateFunction::Max
                    | AggregateFunction::Product
                    | AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor,
                column,
                ..
            },
        ) => Some((table.as_str(), column.as_str())),
        _ => None,
    };
    // Verify that only a named database table can be scanned.
    if !matches!(select.source, Some(Source::Table(_))) || cluster.peers().is_empty() {
        return Response::text(
            "400 Bad Request",
            "no distributed database peers configured\n",
        );
    }
    let table = match &select.source {
        Some(Source::Table(table)) => table.as_str(),
        _ => unreachable!("validated named table"),
    };
    let retained_input = AtomicUsize::new(0);
    // Bound concurrency without wave barriers: a fast peer immediately
    // claims the next pending shard, even if an earlier peer is still slow.
    const MAX_INFLIGHT_SHARDS: usize = 8;
    let results =
        dispatch::map_peers_bounded_failfast(cluster.peers(), MAX_INFLIGHT_SHARDS, |peer| {
            read_shard(
                peer,
                shard_sql.as_str(),
                sum_schema,
                group_schema,
                table,
                sorted_prefix,
                grouped_multi_count_column_query || grouped_count_column_query,
                extrema_function.is_some(),
                &retained_input,
            )
        });
    let mut shards = Vec::with_capacity(results.len());
    let mut sum_type: Option<String> = None;
    let mut group_key_kind: Option<String> = None;
    let mut expected_schema: Option<Vec<u8>> = None;
    for result in results {
        match result {
            Ok(ShardResponse {
                cap,
                body,
                kind,
                key_kind,
                schema,
            }) => {
                if expected_schema
                    .as_ref()
                    .is_some_and(|expected| expected != &schema)
                {
                    return Response::text("409 Conflict", "distributed shard schemas disagree\n");
                }
                if expected_schema.is_none() {
                    expected_schema = Some(schema);
                }
                if let Some(kind) = kind {
                    if sum_type
                        .as_deref()
                        .is_some_and(|existing| existing != kind.as_str())
                    {
                        return Response::text(
                            "409 Conflict",
                            "distributed integer aggregate shard column types disagree\n",
                        );
                    }
                    sum_type = Some(kind);
                }
                if let Some(key_kind) = key_kind {
                    if group_key_kind
                        .as_deref()
                        .is_some_and(|existing| existing != key_kind.as_str())
                    {
                        return Response::text(
                            "409 Conflict",
                            "distributed GROUP BY key types disagree\n",
                        );
                    }
                    group_key_kind = Some(key_kind);
                }
                shards.push((cap, body));
            }
            Err(message) => {
                return Response::text("502 Bad Gateway", &format!("{message}\n"));
            }
        }
    }
    let merged = if count_query {
        sum_shard_counts(&shards)
    } else if let Some(column) = distinct_count_column {
        let kind = expected_schema
            .as_deref()
            .and_then(|schema| validated_schema_kind(schema, column));
        match kind {
            Some(
                kind @ ("Int8" | "UInt8" | "Int16" | "UInt16" | "Nullable(Int8)"
                | "Nullable(UInt8)" | "Nullable(Int16)" | "Nullable(UInt16)"),
            ) => merge_distinct_bitmap16(&shards, column, kind),
            Some("Int32" | "Nullable(Int32)") => merge_distinct_bitmap32(&shards, column, true),
            Some("UInt32" | "Nullable(UInt32)") => merge_distinct_bitmap32(&shards, column, false),
            Some("UUID") => merge_distinct_uuid(&shards, column, false),
            Some("Nullable(UUID)") => merge_distinct_uuid(&shards, column, true),
            Some(
                kind @ ("Int64" | "UInt64" | "Int128" | "UInt128" | "Nullable(Int64)"
                | "Nullable(UInt64)" | "Nullable(Int128)" | "Nullable(UInt128)"),
            ) => merge_distinct_wide_integers(&shards, column, kind),
            _ => merge_distinct_counts(&shards, column),
        }
    } else if multi_grouped_distinct_query {
        match &select.projection {
            Projection::GroupedMultiDistinctCount {
                columns,
                value_column,
            } => merge_multi_grouped_distinct_pairs(
                &shards,
                columns,
                value_column,
                max_rows.min(250_000),
            ),
            _ => Err(MergeError::InvalidGroupedCount),
        }
    } else if grouped_distinct_query {
        match &select.projection {
            Projection::GroupedDistinctCount {
                column,
                value_column,
            } => merge_grouped_distinct_pairs(&shards, column, value_column, max_rows.min(250_000)),
            _ => Err(MergeError::InvalidGroupedCount),
        }
    } else if uniq_query {
        match &select.projection {
            Projection::ApproxDistinct { column } => {
                merge_uniq_shard_states(&shards, column, select.alias.as_deref())
            }
            _ => Err(MergeError::InvalidUniqState),
        }
    } else if sum_query {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidSum), |kind| {
                merge_integer_sums(&shards, kind)
            })
    } else if rms_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::Aggregate { column, .. }, Some(kind)) if integer_type(kind) => {
                rms_merge::merge_global_rms(&shards, column)
            }
            _ => Err(MergeError::InvalidSum),
        }
    } else if avg_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::Aggregate { column, .. }, Some(kind)) => {
                avg_merge::merge_global_avg(&shards, column, kind)
            }
            _ => Err(MergeError::InvalidSum),
        }
    } else if range_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::Aggregate { column, .. }, Some(kind)) => {
                range_merge::merge_global_range(&shards, column, kind)
            }
            _ => Err(MergeError::InvalidExtremum),
        }
    } else if product_query {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidProduct), |kind| {
                merge_integer_products(&shards, kind)
            })
    } else if let Some(function) = bitwise_function {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidBitwise), |kind| {
                merge_integer_bitwise(&shards, kind, function)
            })
    } else if let Some(function) = extrema_function {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidExtremum), |kind| {
                if matches!(kind, "UUID" | "Nullable(UUID)") {
                    uuid_extrema::merge_uuid_extrema(&shards, function)
                } else {
                    merge_integer_extrema(&shards, kind, function)
                }
            })
    } else if grouped_extremum_query || grouped_product_query || grouped_bitwise_query {
        match (
            sum_type.as_deref(),
            group_schema,
            sum_schema,
            &select.projection,
        ) {
            (
                Some(kind),
                Some((_, key)),
                Some((_, value)),
                Projection::GroupedAggregate { function, .. },
            ) => {
                let limit = max_rows.min(250_000);
                match function {
                    AggregateFunction::Min | AggregateFunction::Max => {
                        merge_grouped_integer_aggregate_query(
                            &shards,
                            limit,
                            kind,
                            key,
                            value,
                            *function,
                            Some(&select),
                        )
                    }
                    AggregateFunction::Product => merge_grouped_integer_products_query(
                        &shards,
                        limit,
                        kind,
                        key,
                        value,
                        Some(&select),
                    ),
                    AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor => merge_grouped_integer_bitwise_query(
                        &shards,
                        limit,
                        kind,
                        key,
                        value,
                        *function,
                        Some(&select),
                    ),
                    _ => Err(MergeError::InvalidGroupedSum),
                }
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_sum_query {
        match (sum_type.as_deref(), group_schema, sum_schema) {
            (Some(kind), Some((_, key)), Some((_, value))) => {
                if matches!(
                    select.projection,
                    Projection::GroupedAggregate {
                        function: AggregateFunction::SumSquares,
                        ..
                    }
                ) || select.having.is_some()
                    || select.order_by.is_some()
                    || select.limit.is_some()
                    || select.offset.is_some()
                {
                    merge_grouped_integer_sums_query(
                        &shards,
                        max_rows.min(250_000),
                        kind,
                        key,
                        value,
                        Some(&select),
                    )
                } else {
                    merge_grouped_integer_sums(&shards, max_rows.min(250_000), kind, key, value)
                }
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_sum_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_sum::merge_multi_key_sums(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_extremum_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_extrema::merge_multi_key_extrema(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    items[0].0,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_countif_query {
        match &select.projection {
            Projection::GroupedMultiAggregates { columns, items, .. } => {
                multi_countif::merge_multi_key_countif(
                    &shards,
                    columns,
                    &items[0].1,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_sumsquares_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_sumsquares::merge_multi_key_sumsquares(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_count_column_query {
        match &select.projection {
            Projection::GroupedMultiAggregates { columns, items, .. } => {
                multi_count_column::merge_multi_key_count_column(
                    &shards,
                    columns,
                    &items[0].1,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_count_query {
        match &select.projection {
            Projection::GroupedMultiCount { columns } => {
                merge_grouped_multi_counts(&shards, columns, max_rows.min(250_000))
            }
            _ => Err(MergeError::InvalidGroupedCount),
        }
    } else if grouped_multi_bitwise_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_bitwise::merge_multi_key_bitwise(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    items[0].0,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedBitwise),
        }
    } else if grouped_multi_product_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_product::merge_multi_key_product(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedProduct),
        }
    } else if grouped_count_query {
        let limit = max_rows.min(250_000);
        if select.having.is_some()
            || select.order_by.is_some()
            || select.limit.is_some()
            || select.offset.is_some()
        {
            merge_grouped_counts_query(&shards, limit, Some(&select))
        } else {
            merge_grouped_counts(&shards, limit)
        }
    } else if sorted_query {
        let order = select.order_by.as_ref().expect("validated bounded sort");
        let Projection::Columns(columns) = &select.projection else {
            unreachable!("validated bounded sort");
        };
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidSortedScan), |kind| {
                let names = columns.iter().map(String::as_str).collect::<Vec<_>>();
                if matches!(kind, "String" | "Nullable(String)") {
                    merge_sorted_text_rows_page_with_nulls(
                        &shards,
                        &names,
                        kind,
                        order.descending,
                        order.nulls_first,
                        select.offset.unwrap_or(0),
                        select.limit.unwrap_or(0),
                        max_rows.min(250_000),
                    )
                } else {
                    merge_sorted_integer_rows_page_with_nulls(
                        &shards,
                        &names,
                        kind,
                        order.descending,
                        order.nulls_first,
                        select.offset.unwrap_or(0),
                        select.limit.unwrap_or(0),
                        max_rows.min(250_000),
                    )
                }
            })
    } else if distinct_rows_query {
        merge_distinct_tsv(
            &shards,
            max_rows.min(250_000),
            select.offset.unwrap_or(0),
            select.limit,
        )
    } else if select.limit.is_some() || select.offset.is_some() {
        merge_tsv_page(
            &shards,
            max_rows.min(250_000),
            select.offset.unwrap_or(0),
            select.limit,
        )
    } else {
        merge_tsv(&shards, max_rows.min(250_000))
    };
    match merged {
        Ok(body) => Response {
            status: "200 OK",
            content_type: "text/tab-separated-values; charset=utf-8",
            body,
        },
        Err(MergeError::SchemaMismatch) => {
            Response::text("409 Conflict", "distributed shard headers disagree\n")
        }
        Err(MergeError::ShardRowLimit) => Response::text(
            "409 Conflict",
            "shard reached its result cap; increase peer --max-rows\n",
        ),
        Err(MergeError::CoordinatorLimit) => Response::text(
            "413 Payload Too Large",
            "distributed result exceeds coordinator limits\n",
        ),
        Err(MergeError::InvalidResponse) => {
            Response::text("502 Bad Gateway", "malformed shard TSV response\n")
        }
        Err(MergeError::InvalidCount) => {
            Response::text("502 Bad Gateway", "malformed shard COUNT response\n")
        }
        Err(MergeError::InvalidUniqState) => {
            Response::text("502 Bad Gateway", "malformed shard uniq state\n")
        }
        Err(MergeError::InvalidDistinctCount) => {
            Response::text("502 Bad Gateway", "malformed shard DISTINCT response\n")
        }
        Err(MergeError::InvalidGroupedCount) => {
            Response::text("502 Bad Gateway", "malformed shard GROUP BY response\n")
        }
        Err(MergeError::InvalidGroupedSum) => {
            Response::text("502 Bad Gateway", "malformed shard grouped SUM response\n")
        }
        Err(MergeError::InvalidGroupedProduct) => Response::text(
            "502 Bad Gateway",
            "malformed shard grouped PRODUCT response\n",
        ),
        Err(MergeError::InvalidGroupedBitwise) => Response::text(
            "502 Bad Gateway",
            "malformed shard grouped bitwise response\n",
        ),
        Err(MergeError::InvalidSortedScan) => Response::text(
            "502 Bad Gateway",
            "malformed shard sorted integer response\n",
        ),
        Err(MergeError::InvalidSum) => {
            Response::text("502 Bad Gateway", "malformed shard SUM response\n")
        }
        Err(MergeError::InvalidBitwise) => Response::text(
            "502 Bad Gateway",
            "malformed shard bitwise aggregate response\n",
        ),
        Err(MergeError::InvalidProduct) => {
            Response::text("502 Bad Gateway", "malformed shard PRODUCT response\n")
        }
        Err(MergeError::ProductOverflow) => {
            Response::text("413 Payload Too Large", "distributed PRODUCT overflow\n")
        }
        Err(MergeError::InvalidExtremum) => {
            Response::text("502 Bad Gateway", "malformed shard MIN/MAX response\n")
        }
        Err(MergeError::SumOverflow) => {
            Response::text("413 Payload Too Large", "distributed SUM overflow\n")
        }
        Err(MergeError::CountOverflow) => {
            Response::text("413 Payload Too Large", "distributed count overflow\n")
        }
    }
}
