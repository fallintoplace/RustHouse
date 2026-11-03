//! Bounded read-only JOIN queries through the SQL HTTP interface.

use super::*;
use rusthouse_query::{
    ExecutionError, JoinSelect, Source, bind_join_having_subquery, bind_join_in_subquery,
    execute_correlated_avg, execute_correlated_join, execute_correlated_min,
    execute_correlated_sum, execute_cte_max_join, execute_derived_join_count, execute_join,
    execute_multi_join, execute_nested_join_sum, explain_join,
};

pub(super) fn try_cte_max(
    sql: &str,
    explain: bool,
    format: QueryFormat,
    config: &Config,
) -> Option<Response> {
    match rusthouse_query::has_cte_max_join(sql) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    }
    if config.part.is_some() && config.database.is_some() {
        return Some(Response::text(
            "400 Bad Request",
            "choose --part or --database, not both\n",
        ));
    }
    let Some(database) = config.database.as_deref() else {
        return Some(Response::text(
            "400 Bad Request",
            "CTE JOIN requires a database\n",
        ));
    };
    let mut plan = match rusthouse_query::parse_cte_max_join(sql) {
        Ok(Some(plan)) => plan,
        Ok(None) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    if explain {
        return Some(Response::text(
            "400 Bad Request",
            "CTE JOIN cannot use --explain yet\n",
        ));
    }
    plan.cap_rows(config.max_rows.min(250_000));
    let _lock = match rusthouse_storage::acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Some(Response::text("409 Conflict", &format!("{error}\n"))),
    };
    let inner = match resolve_database_table(database, plan.inner_table()) {
        Ok(path) => path,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    let outer = match resolve_database_table(database, plan.outer_table()) {
        Ok(path) => path,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    Some(format_response(format, |mut output| {
        execute_cte_max_join(&plan, &inner, &outer, &mut output)
    }))
}

pub(super) fn try_derived_count(
    sql: &str,
    explain: bool,
    format: QueryFormat,
    config: &Config,
) -> Option<Response> {
    match rusthouse_query::has_derived_join_count(sql) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    }
    if config.part.is_some() && config.database.is_some() {
        return Some(Response::text(
            "400 Bad Request",
            "choose --part or --database, not both\n",
        ));
    }
    let Some(database) = config.database.as_deref() else {
        return Some(Response::text(
            "400 Bad Request",
            "derived JOIN requires a database\n",
        ));
    };
    let _lock = match rusthouse_storage::acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Some(Response::text("409 Conflict", &format!("{error}\n"))),
    };
    let sum = rusthouse_query::parse_derived_join_sum_with_schema(sql, |table| {
        let path = resolve_database_table(database, table).map_err(rusthouse_query::ParseError)?;
        rusthouse_storage::inspect_query_source(&path)
            .map(|metadata| {
                metadata
                    .columns
                    .into_iter()
                    .map(|column| column.name)
                    .collect()
            })
            .map_err(|error| rusthouse_query::ParseError(error.to_string()))
    });
    match sum {
        Ok(Some(mut query)) => {
            if explain {
                return Some(Response::text(
                    "400 Bad Request",
                    "derived JOIN cannot use --explain yet\n",
                ));
            }
            let cap = config.max_rows.min(250_000);
            query.limit = Some(query.limit.map_or(cap, |limit| limit.min(cap)));
            let tables = [&query.left, &query.right]
                .into_iter()
                .chain(query.steps.iter().map(|step| &step.table));
            let paths = match tables
                .map(|table| resolve_database_table(database, table))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(paths) => paths,
                Err(error) => {
                    return Some(Response::text("400 Bad Request", &format!("{error}\n")));
                }
            };
            return Some(format_response(format, |mut output| {
                if paths.len() == 2 {
                    execute_join(&query, &paths[0], &paths[1], &mut output)
                } else {
                    execute_multi_join(&query, &paths, &mut output)
                }
            }));
        }
        Ok(None) => {}
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    }
    let plan = rusthouse_query::parse_derived_join_count_with_schema(sql, |table| {
        let path = resolve_database_table(database, table).map_err(rusthouse_query::ParseError)?;
        rusthouse_storage::inspect_query_source(&path)
            .map(|metadata| {
                metadata
                    .columns
                    .into_iter()
                    .map(|column| column.name)
                    .collect()
            })
            .map_err(|error| rusthouse_query::ParseError(error.to_string()))
    });
    let mut plan = match plan {
        Ok(Some(plan)) => plan,
        Ok(None) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    if explain {
        return Some(Response::text(
            "400 Bad Request",
            "derived JOIN cannot use --explain yet\n",
        ));
    }
    plan.cap_rows(config.max_rows.min(250_000));
    let inner = plan.inner();
    let left = match resolve_database_table(database, &inner.left) {
        Ok(path) => path,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    let right = match resolve_database_table(database, &inner.right) {
        Ok(path) => path,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    Some(format_response(format, |mut output| {
        execute_derived_join_count(&plan, &left, &right, &mut output)
    }))
}

pub(super) fn try_correlated_avg(
    sql: &str,
    explain: bool,
    format: QueryFormat,
    config: &Config,
) -> Option<Response> {
    match rusthouse_query::has_join_correlated_avg(sql) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    }
    let is_join = match (
        rusthouse_query::has_implicit_join(sql),
        rusthouse_query::has_explicit_join(sql),
    ) {
        (Ok(implicit), Ok(explicit)) => implicit || explicit,
        (Err(error), _) | (_, Err(error)) => {
            return Some(Response::text("400 Bad Request", &format!("{error}\n")));
        }
    };
    if !is_join {
        return None;
    }
    if config.part.is_some() && config.database.is_some() {
        return Some(Response::text(
            "400 Bad Request",
            "choose --part or --database, not both\n",
        ));
    }
    let Some(database) = config.database.as_deref() else {
        return Some(Response::text(
            "400 Bad Request",
            "JOIN requires a database\n",
        ));
    };
    let _lock = match rusthouse_storage::acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Some(Response::text("409 Conflict", &format!("{error}\n"))),
    };
    let plan = rusthouse_query::parse_join_correlated_avg_with_schema(sql, |table| {
        let path = resolve_database_table(database, table).map_err(rusthouse_query::ParseError)?;
        rusthouse_storage::inspect_query_source(&path)
            .map(|metadata| {
                metadata
                    .columns
                    .into_iter()
                    .map(|column| column.name)
                    .collect()
            })
            .map_err(|error| rusthouse_query::ParseError(error.to_string()))
    });
    let mut plan = match plan {
        Ok(Some(plan)) => plan,
        Ok(None) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    if explain {
        return Some(Response::text(
            "400 Bad Request",
            "correlated JOIN AVG cannot use --explain yet\n",
        ));
    }
    plan.cap_rows(config.max_rows.min(250_000));
    let query = plan.outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let paths = match tables
        .map(|table| resolve_database_table(database, table))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) => paths,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    let inner = match resolve_database_table(database, &plan.table) {
        Ok(path) => path,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    Some(format_response(format, |mut output| {
        execute_correlated_avg(&plan, &paths, &inner, &mut output)
    }))
}

pub(super) fn try_nested_sum(
    sql: &str,
    explain: bool,
    format: QueryFormat,
    config: &Config,
) -> Option<Response> {
    match rusthouse_query::has_nested_join_sum(sql) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    }
    let is_join = match (
        rusthouse_query::has_implicit_join(sql),
        rusthouse_query::has_explicit_join(sql),
    ) {
        (Ok(implicit), Ok(explicit)) => implicit || explicit,
        (Err(error), _) | (_, Err(error)) => {
            return Some(Response::text("400 Bad Request", &format!("{error}\n")));
        }
    };
    if !is_join {
        return None;
    }
    if config.part.is_some() && config.database.is_some() {
        return Some(Response::text(
            "400 Bad Request",
            "choose --part or --database, not both\n",
        ));
    }
    let Some(database) = config.database.as_deref() else {
        return Some(Response::text(
            "400 Bad Request",
            "JOIN requires a database\n",
        ));
    };
    let _lock = match rusthouse_storage::acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Some(Response::text("409 Conflict", &format!("{error}\n"))),
    };
    let plan = rusthouse_query::parse_nested_join_sum_with_schema(sql, |table| {
        let path = resolve_database_table(database, table).map_err(rusthouse_query::ParseError)?;
        rusthouse_storage::inspect_query_source(&path)
            .map(|metadata| {
                metadata
                    .columns
                    .into_iter()
                    .map(|column| column.name)
                    .collect()
            })
            .map_err(|error| rusthouse_query::ParseError(error.to_string()))
    });
    let mut plan = match plan {
        Ok(Some(plan)) => plan,
        Ok(None) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    if explain {
        return Some(Response::text(
            "400 Bad Request",
            "nested JOIN SUM cannot use --explain yet\n",
        ));
    }
    plan.cap_rows(config.max_rows.min(250_000));
    let query = plan.outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let paths = match tables
        .map(|table| resolve_database_table(database, table))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) => paths,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    let query = plan.inner().outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let inner = match tables
        .map(|table| resolve_database_table(database, table))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) => paths,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    let sum = match resolve_database_table(database, &plan.inner().table) {
        Ok(path) => path,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    Some(format_response(format, |mut output| {
        execute_nested_join_sum(&plan, &paths, &inner, &sum, &mut output)
    }))
}

pub(super) fn try_correlated_sum(
    sql: &str,
    explain: bool,
    format: QueryFormat,
    config: &Config,
) -> Option<Response> {
    match rusthouse_query::has_join_correlated_sum(sql) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    }
    let is_join = match (
        rusthouse_query::has_implicit_join(sql),
        rusthouse_query::has_explicit_join(sql),
    ) {
        (Ok(implicit), Ok(explicit)) => implicit || explicit,
        (Err(error), _) | (_, Err(error)) => {
            return Some(Response::text("400 Bad Request", &format!("{error}\n")));
        }
    };
    if !is_join {
        return None;
    }
    if config.part.is_some() && config.database.is_some() {
        return Some(Response::text(
            "400 Bad Request",
            "choose --part or --database, not both\n",
        ));
    }
    let Some(database) = config.database.as_deref() else {
        return Some(Response::text(
            "400 Bad Request",
            "JOIN requires a database\n",
        ));
    };
    let _lock = match rusthouse_storage::acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Some(Response::text("409 Conflict", &format!("{error}\n"))),
    };
    let plan = rusthouse_query::parse_join_correlated_sum_with_schema(sql, |table| {
        let path = resolve_database_table(database, table).map_err(rusthouse_query::ParseError)?;
        rusthouse_storage::inspect_query_source(&path)
            .map(|metadata| {
                metadata
                    .columns
                    .into_iter()
                    .map(|column| column.name)
                    .collect()
            })
            .map_err(|error| rusthouse_query::ParseError(error.to_string()))
    });
    let mut plan = match plan {
        Ok(Some(plan)) => plan,
        Ok(None) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    if explain {
        return Some(Response::text(
            "400 Bad Request",
            "correlated JOIN SUM cannot use --explain yet\n",
        ));
    }
    plan.cap_rows(config.max_rows.min(250_000));
    let query = plan.outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let paths = match tables
        .map(|table| resolve_database_table(database, table))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) => paths,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    let inner = match resolve_database_table(database, &plan.table) {
        Ok(path) => path,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    Some(format_response(format, |mut output| {
        execute_correlated_sum(&plan, &paths, &inner, &mut output)
    }))
}

pub(super) fn try_correlated_min(
    sql: &str,
    explain: bool,
    format: QueryFormat,
    config: &Config,
) -> Option<Response> {
    match rusthouse_query::has_join_correlated_min(sql) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    }
    let is_join = match (
        rusthouse_query::has_implicit_join(sql),
        rusthouse_query::has_explicit_join(sql),
    ) {
        (Ok(implicit), Ok(explicit)) => implicit || explicit,
        (Err(error), _) | (_, Err(error)) => {
            return Some(Response::text("400 Bad Request", &format!("{error}\n")));
        }
    };
    if !is_join {
        return None;
    }
    if config.part.is_some() && config.database.is_some() {
        return Some(Response::text(
            "400 Bad Request",
            "choose --part or --database, not both\n",
        ));
    }
    let Some(database) = config.database.as_deref() else {
        return Some(Response::text(
            "400 Bad Request",
            "JOIN requires a database\n",
        ));
    };
    let _lock = match rusthouse_storage::acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Some(Response::text("409 Conflict", &format!("{error}\n"))),
    };
    let plan = rusthouse_query::parse_join_correlated_min_with_schema(sql, |table| {
        let path = resolve_database_table(database, table).map_err(rusthouse_query::ParseError)?;
        rusthouse_storage::inspect_query_source(&path)
            .map(|metadata| {
                metadata
                    .columns
                    .into_iter()
                    .map(|column| column.name)
                    .collect()
            })
            .map_err(|error| rusthouse_query::ParseError(error.to_string()))
    });
    let mut plan = match plan {
        Ok(Some(plan)) => plan,
        Ok(None) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    if explain {
        return Some(Response::text(
            "400 Bad Request",
            "correlated JOIN MIN cannot use --explain yet\n",
        ));
    }
    plan.cap_rows(config.max_rows.min(250_000));
    let query = plan.outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let paths = match tables
        .map(|table| resolve_database_table(database, table))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) => paths,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    let query = plan.inner();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let inner = match tables
        .map(|table| resolve_database_table(database, table))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) => paths,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    Some(format_response(format, |mut output| {
        execute_correlated_min(&plan, &paths, &inner, &mut output)
    }))
}

pub(super) fn try_correlated(
    sql: &str,
    explain: bool,
    format: QueryFormat,
    config: &Config,
) -> Option<Response> {
    match rusthouse_query::has_top_level_exists(sql) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    }
    let is_join = match (
        rusthouse_query::has_implicit_join(sql),
        rusthouse_query::has_explicit_join(sql),
    ) {
        (Ok(implicit), Ok(explicit)) => implicit || explicit,
        (Err(error), _) | (_, Err(error)) => {
            return Some(Response::text("400 Bad Request", &format!("{error}\n")));
        }
    };
    if !is_join {
        return None;
    }
    if config.part.is_some() && config.database.is_some() {
        return Some(Response::text(
            "400 Bad Request",
            "choose --part or --database, not both\n",
        ));
    }
    let Some(database) = config.database.as_deref() else {
        return Some(Response::text(
            "400 Bad Request",
            "JOIN requires a database\n",
        ));
    };
    let _lock = match rusthouse_storage::acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Some(Response::text("409 Conflict", &format!("{error}\n"))),
    };
    let plan = rusthouse_query::parse_join_correlated_exists_with_schema(sql, |table| {
        let path = resolve_database_table(database, table).map_err(rusthouse_query::ParseError)?;
        rusthouse_storage::inspect_query_source(&path)
            .map(|metadata| {
                metadata
                    .columns
                    .into_iter()
                    .map(|column| column.name)
                    .collect()
            })
            .map_err(|error| rusthouse_query::ParseError(error.to_string()))
    });
    let mut plan = match plan {
        Ok(Some(plan)) => plan,
        Ok(None) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    if explain {
        return Some(Response::text(
            "400 Bad Request",
            "correlated JOIN EXISTS cannot use --explain yet\n",
        ));
    }
    plan.cap_rows(config.max_rows.min(250_000));
    let query = plan.outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let paths = match tables
        .map(|table| resolve_database_table(database, table))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) => paths,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    let inner_paths = match plan
        .filters
        .iter()
        .map(|filter| resolve_database_table(database, &filter.table))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) => paths,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    Some(format_response(format, |mut output| {
        execute_correlated_join(&plan, &paths, &inner_paths, &mut output)
    }))
}

fn finish_output<T>(
    result: Result<(), ExecutionError>,
    finish: impl FnOnce() -> std::io::Result<T>,
) -> Result<(), ExecutionError> {
    result.and_then(|()| finish().map(|_| ()).map_err(ExecutionError::Io))
}

pub(super) fn handle(
    query: &mut JoinSelect,
    explain: bool,
    format: QueryFormat,
    config: &Config,
) -> Response {
    if config.part.is_some() && config.database.is_some() {
        return Response::text("400 Bad Request", "choose --part or --database, not both\n");
    }
    let Some(database) = config.database.as_deref() else {
        return Response::text("400 Bad Request", "JOIN requires a database\n");
    };
    let join_row_cap = config.max_rows.min(250_000);
    query.limit = Some(
        query
            .limit
            .map_or(join_row_cap, |limit| limit.min(join_row_cap)),
    );
    if explain {
        return Response::text(
            "200 OK",
            &format!("Logical plan (no data read):\n{}\n", explain_join(query)),
        );
    }
    // Hold one read lock across all source resolution and row decoding.
    let _lock = match rusthouse_storage::acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Response::text("409 Conflict", &format!("{error}\n")),
    };
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let mut paths = Vec::with_capacity(query.steps.len() + 2);
    for table in tables {
        match resolve_database_table(database, table) {
            Ok(path) => paths.push(path),
            Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
        }
    }
    if let Some(inner) = query
        .grouped_sum_having
        .as_ref()
        .and_then(|having| having.subquery.as_ref())
    {
        let tables = [&inner.left, &inner.right]
            .into_iter()
            .chain(inner.steps.iter().map(|step| &step.table));
        let inner_paths = match tables
            .map(|table| resolve_database_table(database, table))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(paths) => paths,
            Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
        };
        if let Err(error) = bind_join_having_subquery(query, &inner_paths) {
            return Response::text("400 Bad Request", &format!("{error}\n"));
        }
    }
    if let Some(subquery) = &query.in_subquery {
        let Some(Source::Table(table)) = &subquery.inner.source else {
            return Response::text(
                "400 Bad Request",
                "JOIN IN subquery requires a stored table\n",
            );
        };
        let path = match resolve_database_table(database, table) {
            Ok(path) => path,
            Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
        };
        if let Err(error) = bind_join_in_subquery(query, &path, &paths) {
            return Response::text("400 Bad Request", &format!("{error}\n"));
        }
    }
    let run = |mut output: &mut dyn Write| -> Result<(), ExecutionError> {
        if paths.len() == 2 {
            execute_join(query, &paths[0], &paths[1], &mut output)
        } else {
            execute_multi_join(query, &paths, &mut output)
        }
    };
    format_response(format, run)
}

fn format_response(
    format: QueryFormat,
    run: impl Fn(&mut dyn Write) -> Result<(), ExecutionError>,
) -> Response {
    let mut writer = LimitedWriter::new();
    let result = match format {
        QueryFormat::Tsv | QueryFormat::TsvNoHeader => run(&mut writer),
        QueryFormat::Csv => run(&mut CsvWriter::new(&mut writer)),
        QueryFormat::CsvNoHeader => run(&mut CsvWriter::new(WithoutHeader::new(&mut writer))),
        QueryFormat::Jsonl => run(&mut JsonlWriter::new(&mut writer)),
        QueryFormat::Markdown => run(&mut MarkdownWriter::new(&mut writer)),
        QueryFormat::SqlValues => {
            let mut values = SqlValuesWriter::new(&mut writer);
            finish_output(run(&mut values), || values.finish())
        }
        QueryFormat::JsonCompactEachRow => {
            let mut compact = JsonCompactEachRowWriter::new(&mut writer);
            finish_output(run(&mut compact), || compact.finish())
        }
        QueryFormat::Json | QueryFormat::JsonPretty => {
            let mut array = if matches!(format, QueryFormat::JsonPretty) {
                JsonArraySink::new_pretty(&mut writer)
            } else {
                JsonArraySink::new(&mut writer)
            };
            let result = run(&mut JsonlWriter::new(&mut array));
            finish_output(result, || array.finish())
        }
    };
    if result.is_ok() && matches!(format, QueryFormat::TsvNoHeader) {
        let header_len = writer
            .data
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(writer.data.len(), |index| index + 1);
        writer.data.drain(..header_len);
    }
    match result {
        Ok(()) => Response {
            status: "200 OK",
            content_type: match format {
                QueryFormat::Tsv | QueryFormat::TsvNoHeader => {
                    "text/tab-separated-values; charset=utf-8"
                }
                QueryFormat::Csv | QueryFormat::CsvNoHeader => "text/csv; charset=utf-8",
                QueryFormat::Jsonl | QueryFormat::JsonCompactEachRow => {
                    "application/x-ndjson; charset=utf-8"
                }
                QueryFormat::Json | QueryFormat::JsonPretty => "application/json; charset=utf-8",
                QueryFormat::SqlValues => "text/plain; charset=utf-8",
                QueryFormat::Markdown => "text/markdown; charset=utf-8",
            },
            body: writer.data,
        },
        Err(_) if writer.exceeded => {
            Response::text("413 Payload Too Large", "SQL response exceeds 16 MiB\n")
        }
        Err(error) => Response::text("400 Bad Request", &format!("{error}\n")),
    }
}

pub(super) fn try_substring_antijoin(
    sql: &str,
    explain: bool,
    format: QueryFormat,
    config: &Config,
) -> Option<Response> {
    match rusthouse_query::has_substring_antijoin(sql) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    }
    if config.part.is_some() && config.database.is_some() {
        return Some(Response::text(
            "400 Bad Request",
            "choose --part or --database, not both\n",
        ));
    }
    let Some(database) = config.database.as_deref() else {
        return Some(Response::text(
            "400 Bad Request",
            "substring anti-join requires a database\n",
        ));
    };
    let _lock = match rusthouse_storage::acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Some(Response::text("409 Conflict", &format!("{error}\n"))),
    };
    let plan = rusthouse_query::parse_substring_antijoin_with_schema(sql, |table| {
        let path = resolve_database_table(database, table).map_err(rusthouse_query::ParseError)?;
        rusthouse_storage::inspect_query_source(&path)
            .map(|metadata| {
                metadata
                    .columns
                    .into_iter()
                    .map(|column| column.name)
                    .collect()
            })
            .map_err(|error| rusthouse_query::ParseError(error.to_string()))
    });
    let mut plan = match plan {
        Ok(Some(plan)) => plan,
        Ok(None) => return None,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    if explain {
        return Some(Response::text(
            "400 Bad Request",
            "substring anti-join cannot use --explain yet\n",
        ));
    }
    plan.cap_rows(config.max_rows.min(250_000));
    let source = match resolve_database_table(database, &plan.table) {
        Ok(path) => path,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    let lookup = match resolve_database_table(database, &plan.lookup_table) {
        Ok(path) => path,
        Err(error) => return Some(Response::text("400 Bad Request", &format!("{error}\n"))),
    };
    Some(format_response(format, |mut output| {
        rusthouse_query::execute_substring_antijoin(&plan, &source, &lookup, &mut output)
    }))
}
