//! Minimal read-only SQL HTTP interface, bound to loopback by the binary.
//! This is intentionally not a full database wire protocol.

mod http_encoding;

use self::http_encoding::*;

pub mod cluster;
mod csv;
mod csv_no_header;
pub mod distributed;
pub mod health;
mod join_query;
mod json_array;
mod json_compact_each_row;
mod jsonl;
mod markdown;
mod metadata_sql;
pub mod remote;
mod sql_values;
mod tables;

use csv::CsvWriter;
use csv_no_header::WithoutHeader;
use json_array::JsonArraySink;
use json_compact_each_row::JsonCompactEachRowWriter;
use jsonl::JsonlWriter;
use markdown::MarkdownWriter;
use rusthouse_query::{
    Source, execute, explain_select, parse_join, parse_select, split_clickhouse_format,
};
use rusthouse_storage::{inspect_query_source, resolve_database_table};
use sql_values::SqlValuesWriter;
use std::io::{self, Write};
use std::path::PathBuf;

pub const MAX_QUERY_BYTES: usize = 64 * 1024;
pub const MAX_HEADER_BYTES: usize = 8192;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CONNECTIONS: usize = 16;

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub part: Option<PathBuf>,
    pub database: Option<PathBuf>,
    pub max_rows: usize,
}

#[derive(Debug)]
pub struct Response {
    pub status: &'static str,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(status: &'static str, message: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: message.as_bytes().to_vec(),
        }
    }
}

struct LimitedWriter {
    data: Vec<u8>,
    exceeded: bool,
}

impl LimitedWriter {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            exceeded: false,
        }
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let new_size = self.data.len().checked_add(bytes.len()).ok_or_else(|| {
            self.exceeded = true;
            io::Error::new(io::ErrorKind::InvalidData, "response size overflow")
        })?;
        if new_size > MAX_RESPONSE_BYTES {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SQL response exceeds 16 MiB",
            ));
        }
        self.data.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn schema_response(config: &Config, table: Option<&str>) -> Response {
    if config.part.is_some() && config.database.is_some() {
        return Response::text("400 Bad Request", "choose --part or --database, not both\n");
    }
    // Hold the same shared lock as SQL queries while resolving the snapshot
    // and building the response. Concurrent DROP/RENAME must not remove it.
    let _database_read_lock = if table.is_some() {
        let Some(database) = config.database.as_deref() else {
            return Response::text("400 Bad Request", "table discovery requires --database\n");
        };
        match rusthouse_storage::acquire_database_lock(database, false) {
            Ok(lock) => Some(lock),
            Err(error) => return Response::text("409 Conflict", &format!("{error}\n")),
        }
    } else {
        None
    };
    let part = if let Some(table) = table {
        let database = config.database.as_deref().expect("checked --database");
        match resolve_database_table(database, table) {
            Ok(part) => part,
            Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
        }
    } else {
        let Some(part) = config.part.as_ref() else {
            return Response::text("400 Bad Request", "no --part source configured\n");
        };
        part.clone()
    };
    let metadata = match inspect_query_source(&part) {
        Ok(metadata) => metadata,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    let mut output = LimitedWriter::new();
    let result = (|| -> io::Result<()> {
        write!(&mut output, "{{\"rows\":{},\"columns\":[", metadata.rows)?;
        for (index, column) in metadata.columns.iter().enumerate() {
            if index > 0 {
                output.write_all(b",")?;
            }
            output.write_all(b"{\"name\":")?;
            write_json_string(&mut output, &column.name)?;
            output.write_all(b",\"type\":")?;
            write_json_string(&mut output, &column.data_type)?;
            output.write_all(b"}")?;
        }
        output.write_all(b"]}\n")
    })();
    match result {
        Ok(()) => Response {
            status: "200 OK",
            content_type: "application/json; charset=utf-8",
            body: output.data,
        },
        Err(_) => Response::text("413 Payload Too Large", "schema response exceeds 16 MiB\n"),
    }
}

#[derive(Clone, Copy)]
enum QueryFormat {
    Tsv,
    TsvNoHeader,
    Csv,
    CsvNoHeader,
    Jsonl,
    Json,
    JsonPretty,
    JsonCompactEachRow,
    SqlValues,
    Markdown,
}

impl QueryFormat {
    fn parse(value: &str) -> Result<Self, &'static str> {
        Ok(match value {
            "tsv" => Self::Tsv,
            "TabSeparatedWithNames" => Self::Tsv,
            "tsv-no-header" => Self::TsvNoHeader,
            "TabSeparated" => Self::TsvNoHeader,
            "csv" => Self::Csv,
            "CSVWithNames" => Self::Csv,
            "csv-no-header" => Self::CsvNoHeader,
            "CSV" => Self::CsvNoHeader,
            "jsonl" => Self::Jsonl,
            "JSONStringsEachRow" => Self::Jsonl,
            "json" => Self::Json,
            "json-pretty" => Self::JsonPretty,
            "json-compact-each-row" => Self::JsonCompactEachRow,
            "JSONCompactStringsEachRow" => Self::JsonCompactEachRow,
            "sql-values" => Self::SqlValues,
            "markdown" => Self::Markdown,
            _ => return Err("unsupported query format"),
        })
    }
}

type QueryParameters = (
    String,
    Option<String>,
    Option<QueryFormat>,
    Option<QueryFormat>,
);

fn query_parameters(target: &str) -> Result<QueryParameters, &'static str> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" && path != "/explain" {
        return Err("unknown SQL endpoint");
    }
    let mut statement = None;
    let mut format = None;
    let mut default_format = None;
    for parameter in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = parameter
            .split_once('=')
            .ok_or("invalid URL query parameter")?;
        match key {
            "query" => {
                if statement.is_some() {
                    return Err("duplicate query parameter");
                }
                statement = Some(decode_url(value)?);
            }
            "format" => {
                if format.is_some() {
                    return Err("duplicate format parameter");
                }
                format = Some(QueryFormat::parse(&decode_url(value)?)?);
            }
            "default_format" => {
                if default_format.is_some() {
                    return Err("duplicate default_format parameter");
                }
                default_format = Some(QueryFormat::parse(&decode_url(value)?)?);
            }
            _ => return Err("unsupported URL query parameter"),
        }
    }
    if path == "/explain" && (format.is_some() || default_format.is_some()) {
        return Err("format is unsupported for EXPLAIN");
    }
    Ok((path.to_owned(), statement, format, default_format))
}

/// Discover the server's read-only mode and resource limits without opening files.
fn capabilities_response(config: &Config) -> Response {
    let source = match (config.part.is_some(), config.database.is_some()) {
        (false, false) => "generated",
        (true, false) => "part",
        (false, true) => "database",
        (true, true) => "invalid",
    };
    let body = format!(
        "{{\"name\":\"RustHouse\",\"version\":\"{}\",\"source\":\"{source}\",\"read_only\":true,\"max_query_bytes\":{MAX_QUERY_BYTES},\"max_response_bytes\":{MAX_RESPONSE_BYTES},\"max_rows\":{}}}\n",
        env!("CARGO_PKG_VERSION"),
        config.max_rows.min(250_000),
    );
    Response {
        status: "200 OK",
        content_type: "application/json; charset=utf-8",
        body: body.into_bytes(),
    }
}

/// Report whether the configured SQL source is a readable real directory.
/// This is a liveness/readiness hint, not a checksum or replica consistency audit.
fn readiness_response(config: &Config) -> Response {
    let source = match (config.part.as_deref(), config.database.as_deref()) {
        (Some(_), Some(_)) => {
            return Response::text("503 Service Unavailable", "conflicting SQL sources\n");
        }
        (Some(path), None) | (None, Some(path)) => path,
        (None, None) => return Response::text("200 OK", "Ready.\n"),
    };
    let readable = std::fs::symlink_metadata(source)
        .is_ok_and(|metadata| metadata.file_type().is_dir())
        && std::fs::read_dir(source).is_ok();
    if readable {
        Response::text("200 OK", "Ready.\n")
    } else {
        Response::text("503 Service Unavailable", "SQL source is not readable\n")
    }
}

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

#[cfg(test)]
#[path = "lib_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "lib_tests/json_string_bulk_perf_tests.rs"]
mod json_string_bulk_perf_tests;

#[cfg(test)]
#[path = "lib_tests/inline_clickhouse_format_tests.rs"]
mod inline_clickhouse_format_tests;

#[cfg(test)]
#[path = "lib_tests/streaming_tsv_header_tests.rs"]
mod streaming_tsv_header_tests;

#[cfg(test)]
#[path = "lib_tests/readiness_endpoint_tests.rs"]
mod readiness_endpoint_tests;
