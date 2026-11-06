//! Keep HTTP routing and SQL request dispatch separate from response types and format writers.

use super::*;

/// Runs only the subset accepted by the existing SELECT parser. DDL and
/// writes cannot pass this interface. All responses are bounded in memory.
pub fn handle_request(method: &str, target: &str, body: &[u8], config: &Config) -> Response {
    if target == "/ping" {
        if method != "GET" {
            return Response::text("405 Method Not Allowed", "ping endpoint requires GET\n");
        }
        if !body.is_empty() {
            return Response::text("400 Bad Request", "GET must not contain a request body\n");
        }
        return Response::text("200 OK", "Ok.\n");
    }
    if target == "/ready" {
        if method != "GET" {
            return Response::text("405 Method Not Allowed", "readiness requires GET\n");
        }
        if !body.is_empty() {
            return Response::text("400 Bad Request", "GET must not contain a request body\n");
        }
        return readiness_response(config);
    }
    if target == "/capabilities" {
        if method != "GET" {
            return Response::text(
                "405 Method Not Allowed",
                "capabilities endpoint requires GET\n",
            );
        }
        if !body.is_empty() {
            return Response::text("400 Bad Request", "GET must not contain a request body\n");
        }
        return capabilities_response(config);
    }
    if target == "/tables" {
        return tables::handle(method, body, config);
    }
    if target == "/schema" || target.starts_with("/schema?") {
        if method != "GET" {
            return Response::text("405 Method Not Allowed", "schema endpoint requires GET\n");
        }
        if !body.is_empty() {
            return Response::text("400 Bad Request", "GET must not contain a request body\n");
        }
        if target.len() > MAX_HEADER_BYTES {
            return Response::text("413 Payload Too Large", "request target too large\n");
        }
        let table = if target == "/schema" {
            None
        } else {
            let Some(value) = target.strip_prefix("/schema?table=") else {
                return Response::text("400 Bad Request", "schema expects one table parameter\n");
            };
            if value.is_empty() || value.contains('&') {
                return Response::text("400 Bad Request", "schema expects one table parameter\n");
            }
            match decode_url(value) {
                Ok(name) => Some(name),
                Err(message) => return Response::text("400 Bad Request", message),
            }
        };
        return schema_response(config, table.as_deref());
    }
    if method != "GET" && method != "POST" {
        return Response::text(
            "405 Method Not Allowed",
            "only GET and POST are supported\n",
        );
    }
    if body.len() > MAX_QUERY_BYTES || target.len() > MAX_HEADER_BYTES {
        return Response::text("413 Payload Too Large", "SQL statement too large\n");
    }
    let (path, query, request_format, default_format) = match query_parameters(target) {
        Ok(v) => v,
        Err(message) => return Response::text("400 Bad Request", message),
    };
    let sql = match method {
        "GET" if !body.is_empty() => {
            return Response::text("400 Bad Request", "GET must not contain a request body\n");
        }
        "GET" => query.unwrap_or_default(),
        // ClickHouse clients may place the beginning of the statement in the
        // query URL parameter and the remainder in the POST body. The separator
        // is a newline, not an empty string: "SEL" + "ECT" must not turn
        // into "SELECT". Validate the combined limit before allocating.
        "POST" => {
            let tail = match std::str::from_utf8(body) {
                Ok(statement) => statement,
                Err(_) => return Response::text("400 Bad Request", "SQL body is not UTF-8\n"),
            };
            match query {
                Some(head) if !head.is_empty() && !tail.is_empty() => {
                    let combined_bytes = head
                        .len()
                        .checked_add(tail.len())
                        .and_then(|size| size.checked_add(1));
                    if combined_bytes.is_none_or(|size| size > MAX_QUERY_BYTES) {
                        return Response::text(
                            "413 Payload Too Large",
                            "SQL statement too large\n",
                        );
                    }
                    format!("{head}\n{tail}")
                }
                Some(head) if !head.is_empty() => head,
                _ => tail.to_owned(),
            }
        }
        _ => unreachable!(),
    };
    if sql.is_empty() || sql.len() > MAX_QUERY_BYTES {
        return Response::text("400 Bad Request", "missing or oversized SQL statement\n");
    }
    let (statement, inline_format) = match split_clickhouse_format(&sql) {
        Ok(parts) => parts,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    let format = if let Some(name) = inline_format {
        if path == "/explain" || request_format.is_some() {
            return Response::text(
                "400 Bad Request",
                "SQL FORMAT cannot be combined with EXPLAIN or URL format\n",
            );
        }
        match QueryFormat::parse(name) {
            Ok(value) => value,
            Err(message) => return Response::text("400 Bad Request", message),
        }
    } else {
        request_format
            .or(default_format)
            .unwrap_or(QueryFormat::Tsv)
    };
    let sql = statement.to_owned();
    // DESCRIBE uses the same read lock and schema validation as SQL SELECT.
    if sql
        .trim_start()
        .split_ascii_whitespace()
        .next()
        .is_some_and(|word| {
            word.eq_ignore_ascii_case("DESCRIBE") || word.eq_ignore_ascii_case("DESC")
        })
    {
        match rusthouse_query::parse_ddl(&sql) {
            Ok(Some(rusthouse_query::Ddl::DescribeTable { name })) if path != "/explain" => {
                return metadata_sql::handle(config, format, &name);
            }
            Ok(_) => return Response::text("400 Bad Request", "unsupported schema SQL\n"),
            Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
        }
    }
    // Discovery SQL uses the same validated schemas and read lock as /tables.
    if sql
        .trim_start()
        .get(..4)
        .is_some_and(|word| word.eq_ignore_ascii_case("SHOW"))
    {
        match rusthouse_query::parse_ddl(&sql) {
            Ok(Some(rusthouse_query::Ddl::ShowTables)) if path != "/explain" => {
                return tables::handle_sql(config, format);
            }
            Ok(_) => return Response::text("400 Bad Request", "unsupported metadata SQL\n"),
            Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
        }
    }
    // Join queries need multiple resolved table sources and an independent
    // bounded executor. Delegate before the single-source SELECT parser.
    if let Some(response) = join_query::try_cte_max(&sql, path == "/explain", format, config) {
        return response;
    }
    if let Some(response) =
        join_query::try_substring_antijoin(&sql, path == "/explain", format, config)
    {
        return response;
    }
    if let Some(response) = join_query::try_derived_count(&sql, path == "/explain", format, config)
    {
        return response;
    }
    if let Some(response) = join_query::try_nested_sum(&sql, path == "/explain", format, config) {
        return response;
    }
    if let Some(response) = join_query::try_correlated_sum(&sql, path == "/explain", format, config)
    {
        return response;
    }
    if let Some(response) = join_query::try_correlated_min(&sql, path == "/explain", format, config)
    {
        return response;
    }
    if let Some(response) = join_query::try_correlated_avg(&sql, path == "/explain", format, config)
    {
        return response;
    }
    if let Some(response) = join_query::try_correlated(&sql, path == "/explain", format, config) {
        return response;
    }
    match parse_join(&sql) {
        Ok(Some(mut join)) => {
            return join_query::handle(&mut join, path == "/explain", format, config);
        }
        Ok(None) => {}
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    }
    let mut select = match parse_select(&sql) {
        Ok(query) => query,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    let cap = config.max_rows.min(250_000);
    select.limit = Some(select.limit.map_or(cap, |limit| limit.min(cap)));
    if path == "/explain" {
        return Response::text(
            "200 OK",
            &format!(
                "Logical plan (no data read):\n{}\n",
                explain_select(&select)
            ),
        );
    }
    if config.part.is_some() && config.database.is_some() {
        return Response::text("400 Bad Request", "choose --part or --database, not both\n");
    }
    // Match the CLI read contract: DROP/RENAME cannot remove parts mid-query.
    // Retain this guard through the entire execution and response conversion.
    let _database_read_lock = match (config.database.as_deref(), select.source.as_ref()) {
        (Some(database), Some(Source::Table(_))) => {
            match rusthouse_storage::acquire_database_lock(database, false) {
                Ok(lock) => Some(lock),
                Err(error) => return Response::text("409 Conflict", &format!("{error}\n")),
            }
        }
        _ => None,
    };
    let mut part = config.part.clone();
    if let (Some(database), Some(Source::Table(table))) =
        (config.database.as_deref(), select.source.as_ref())
    {
        part = match resolve_database_table(database, table) {
            Ok(path) => Some(path),
            Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
        };
        select.source = Some(Source::Table("part".into()));
    }
    let mut writer = LimitedWriter::new();
    let result = match format {
        QueryFormat::Tsv => execute(&select, part.as_deref(), &mut writer),
        QueryFormat::TsvNoHeader => {
            let mut without_header = WithoutHeader::new(&mut writer);
            execute(&select, part.as_deref(), &mut without_header)
        }
        QueryFormat::Csv => {
            let mut csv = CsvWriter::new(&mut writer);
            execute(&select, part.as_deref(), &mut csv)
        }
        QueryFormat::CsvNoHeader => {
            let mut csv = CsvWriter::new(WithoutHeader::new(&mut writer));
            execute(&select, part.as_deref(), &mut csv)
        }
        QueryFormat::SqlValues => {
            let mut values = SqlValuesWriter::new(&mut writer);
            execute(&select, part.as_deref(), &mut values)
                .and_then(|()| values.finish().map_err(rusthouse_query::ExecutionError::Io))
        }
        QueryFormat::Markdown => {
            let mut markdown = MarkdownWriter::new(&mut writer);
            execute(&select, part.as_deref(), &mut markdown)
        }
        QueryFormat::Jsonl => {
            let mut jsonl = JsonlWriter::new(&mut writer);
            execute(&select, part.as_deref(), &mut jsonl)
        }
        QueryFormat::JsonCompactEachRow => {
            let mut compact = JsonCompactEachRowWriter::new(&mut writer);
            execute(&select, part.as_deref(), &mut compact).and_then(|()| {
                compact
                    .finish()
                    .map(|_| ())
                    .map_err(rusthouse_query::ExecutionError::Io)
            })
        }
        QueryFormat::Json | QueryFormat::JsonPretty => {
            let mut array = if matches!(format, QueryFormat::JsonPretty) {
                JsonArraySink::new_pretty(&mut writer)
            } else {
                JsonArraySink::new(&mut writer)
            };
            let result = {
                let mut jsonl = JsonlWriter::new(&mut array);
                execute(&select, part.as_deref(), &mut jsonl)
            };
            result.and_then(|()| {
                array
                    .finish()
                    .map(|_| ())
                    .map_err(rusthouse_query::ExecutionError::Io)
            })
        }
    };
    match result {
        Ok(()) => Response {
            status: "200 OK",
            content_type: match format {
                QueryFormat::Tsv | QueryFormat::TsvNoHeader => {
                    "text/tab-separated-values; charset=utf-8"
                }
                QueryFormat::Csv | QueryFormat::CsvNoHeader => "text/csv; charset=utf-8",
                QueryFormat::Jsonl => "application/x-ndjson; charset=utf-8",
                QueryFormat::Json | QueryFormat::JsonPretty => "application/json; charset=utf-8",
                QueryFormat::JsonCompactEachRow => "application/x-ndjson; charset=utf-8",
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
