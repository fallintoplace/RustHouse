//! Minimal read-only SQL HTTP interface, bound to loopback by the binary.
//! This is intentionally not a full database wire protocol.

mod http_encoding;

use self::http_encoding::*;

pub mod cluster;
mod csv;
mod csv_no_header;
pub mod distributed;
pub mod health;
mod json_array;
mod json_compact_each_row;
mod jsonl;
mod markdown;
pub mod remote;
mod sql_values;
mod tables;

use csv::CsvWriter;
use csv_no_header::WithoutHeader;
use json_array::JsonArraySink;
use json_compact_each_row::JsonCompactEachRowWriter;
use jsonl::JsonlWriter;
use markdown::MarkdownWriter;
use rusthouse_query::{Source, execute, explain_select, parse_select};
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
            "sql-values" => Self::SqlValues,
            "markdown" => Self::Markdown,
            _ => return Err("unsupported query format"),
        })
    }
}

fn query_parameters(target: &str) -> Result<(String, Option<String>, QueryFormat), &'static str> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" && path != "/explain" {
        return Err("unknown SQL endpoint");
    }
    let mut statement = None;
    let mut format = None;
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
            _ => return Err("unsupported URL query parameter"),
        }
    }
    if path == "/explain" && format.is_some() {
        return Err("format is unsupported for EXPLAIN");
    }
    Ok((
        path.to_owned(),
        statement,
        format.unwrap_or(QueryFormat::Tsv),
    ))
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
    let (path, query, format) = match query_parameters(target) {
        Ok(v) => v,
        Err(message) => return Response::text("400 Bad Request", message),
    };
    let sql = match method {
        "GET" if !body.is_empty() => {
            return Response::text("400 Bad Request", "GET must not contain a request body\n");
        }
        "GET" => query.unwrap_or_default(),
        "POST" if query.is_some() => {
            return Response::text(
                "400 Bad Request",
                "do not combine query URL and POST body\n",
            );
        }
        "POST" => match std::str::from_utf8(body) {
            Ok(statement) => statement.to_owned(),
            Err(_) => return Response::text("400 Bad Request", "SQL body is not UTF-8\n"),
        },
        _ => unreachable!(),
    };
    if sql.is_empty() || sql.len() > MAX_QUERY_BYTES {
        return Response::text("400 Bad Request", "missing or oversized SQL statement\n");
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
        QueryFormat::Tsv | QueryFormat::TsvNoHeader => {
            execute(&select, part.as_deref(), &mut writer)
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
    if result.is_ok() && matches!(format, QueryFormat::TsvNoHeader) {
        let header_bytes = writer
            .data
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(writer.data.len(), |index| index + 1);
        writer.data.drain(..header_bytes);
    }
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
mod tests {
    use super::*;
    use std::fs;

    fn config() -> Config {
        Config {
            part: None,
            database: None,
            max_rows: 3,
        }
    }

    #[test]
    fn database_mode_reads_only_visible_matching_tables() {
        use rusthouse_storage::{Column, write_wide_part};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

        let database = std::env::temp_dir().join(format!(
            "rusthouse-server-db-{}-{}",
            std::process::id(),
            NEXT_DB.fetch_add(1, Ordering::Relaxed)
        ));
        let parts = database.join("events.parts");
        fs::create_dir_all(&parts).unwrap();
        let columns = vec![
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "name".into(),
                data_type: "String".into(),
            },
        ];
        write_wide_part(
            &parts.join("all_0_0_0"),
            &columns,
            &[
                vec![Some("2".into()), Some("Grace".into())],
                vec![Some("1".into()), Some("Ada".into())],
            ],
        )
        .unwrap();
        fs::write(
            database.join("events.schema"),
            "rusthouse-schema-v1\nid\tUInt64\nname\tString\n",
        )
        .unwrap();
        let config = Config {
            part: None,
            database: Some(database.clone()),
            max_rows: 10,
        };
        let schema = handle_request("GET", "/schema?table=events", b"", &config);
        assert_eq!(schema.status, "200 OK");
        assert_eq!(schema.content_type, "application/json; charset=utf-8");
        assert_eq!(
            schema.body,
            b"{\"rows\":2,\"columns\":[{\"name\":\"id\",\"type\":\"UInt64\"},{\"name\":\"name\",\"type\":\"String\"}]}\n"
        );
        let result = handle_request(
            "POST",
            "/",
            b"SELECT id, name FROM events ORDER BY id",
            &config,
        );
        assert_eq!(result.status, "200 OK");
        assert_eq!(result.body, b"id\tname\n1\tAda\n2\tGrace\n");
        let count = handle_request("POST", "/", b"SELECT count(*) FROM events", &config);
        assert_eq!(count.body, b"count\n2\n");
        // SQL over HTTP must follow the same active generation as the CLI.
        let generation = parts.join(".rusthouse-versions").join("g1");
        fs::create_dir_all(&generation).unwrap();
        write_wide_part(
            &generation.join("all_1_1_0"),
            &columns,
            &[vec![Some("3".into()), Some("Lin".into())]],
        )
        .unwrap();
        fs::write(parts.join(".rusthouse-active"), "g1\n").unwrap();
        let current = handle_request(
            "POST",
            "/",
            b"SELECT id, name FROM events ORDER BY id",
            &config,
        );
        assert_eq!(current.status, "200 OK");
        assert_eq!(current.body, b"id\tname\n3\tLin\n");
        assert!(
            handle_request("GET", "/schema?table=events", b"", &config)
                .body
                .starts_with(b"{\"rows\":1,")
        );
        assert_eq!(
            handle_request("POST", "/", b"SELECT count(*) FROM events", &config).body,
            b"count\n1\n"
        );
        fs::write(parts.join(".rusthouse-active"), "g../old\n").unwrap();
        assert_eq!(
            handle_request("POST", "/", b"SELECT id FROM events", &config).status,
            "400 Bad Request"
        );
        fs::write(parts.join(".rusthouse-active"), "g1\n").unwrap();
        let mutation = handle_request("POST", "/", b"DROP TABLE events", &config);
        assert_eq!(mutation.status, "400 Bad Request");
        assert_eq!(
            handle_request("POST", "/", b"SELECT id FROM missing", &config).status,
            "400 Bad Request"
        );
        fs::write(
            database.join("events.schema"),
            "rusthouse-schema-v1\nid\tUInt8\nname\tString\n",
        )
        .unwrap();
        assert_eq!(
            handle_request("POST", "/", b"SELECT id FROM events", &config).status,
            "400 Bad Request"
        );
        fs::remove_file(database.join("events.schema")).unwrap();
        assert_eq!(
            handle_request("POST", "/", b"SELECT id FROM events", &config).status,
            "400 Bad Request"
        );
        fs::remove_dir_all(database).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn database_http_query_respects_cli_exclusive_lock() {
        use rusthouse_storage::{Column, acquire_database_lock, write_wide_part};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

        let database = std::env::temp_dir().join(format!(
            "rusthouse-server-lock-{}-{}",
            std::process::id(),
            NEXT_DB.fetch_add(1, Ordering::Relaxed)
        ));
        let parts = database.join("events.parts");
        fs::create_dir_all(&parts).unwrap();
        write_wide_part(
            &parts.join("all_0_0_0"),
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("5".into())]],
        )
        .unwrap();
        fs::write(
            database.join("events.schema"),
            "rusthouse-schema-v1\nid\tUInt64\n",
        )
        .unwrap();
        let config = Config {
            database: Some(database.clone()),
            part: None,
            max_rows: 10,
        };

        let writer = acquire_database_lock(&database, true).unwrap();
        let blocked = handle_request("POST", "/", b"SELECT id FROM events", &config);
        assert_eq!(blocked.status, "409 Conflict");
        assert_eq!(
            handle_request("GET", "/schema?table=events", b"", &config).status,
            "409 Conflict"
        );
        drop(writer);

        let allowed = handle_request("POST", "/", b"SELECT id FROM events", &config);
        assert_eq!(allowed.status, "200 OK");
        assert_eq!(allowed.body, b"id\n5\n");
        fs::remove_dir_all(database).unwrap();
    }

    #[test]
    fn rejects_invalid_ping_methods_and_bodies() {
        assert_eq!(
            handle_request("GET", "/ping", b"payload", &config()).status,
            "400 Bad Request"
        );
        assert_eq!(
            handle_request("POST", "/ping", b"", &config()).status,
            "405 Method Not Allowed"
        );
        assert_eq!(
            handle_request("PUT", "/ping", b"", &config()).status,
            "405 Method Not Allowed"
        );
    }

    #[test]
    fn accepts_post_sql_above_four_kib_but_rejects_oversized_bodies() {
        let statement = format!("SELECT number FROM numbers(1){}", " ".repeat(5000));
        let response = handle_request("POST", "/", statement.as_bytes(), &config());
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.body, b"number\n0\n");
        let too_large = format!("SELECT 1{}", " ".repeat(MAX_QUERY_BYTES));
        assert_eq!(
            handle_request("POST", "/", too_large.as_bytes(), &config()).status,
            "413 Payload Too Large"
        );
    }

    #[test]
    fn serves_health_and_bounded_select() {
        assert_eq!(
            handle_request("GET", "/ping", b"", &config()).body,
            b"Ok.\n"
        );
        let response = handle_request(
            "GET",
            "/?query=SELECT+number+FROM+numbers%2810%29",
            b"",
            &config(),
        );
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.body, b"number\n0\n1\n2\n");
    }

    #[test]
    fn can_return_more_than_one_hundred_thousand_rows() {
        let config = Config {
            max_rows: 100_001,
            ..config()
        };
        let response = handle_request("POST", "/", b"SELECT number FROM numbers(100001)", &config);
        assert_eq!(response.status, "200 OK");
        assert_eq!(
            response.body.iter().filter(|&&byte| byte == b'\n').count(),
            100_002
        );
    }

    #[test]
    fn advertises_capabilities_without_opening_source_files() {
        let default = handle_request("GET", "/capabilities", b"", &config());
        assert_eq!(default.status, "200 OK");
        assert_eq!(default.content_type, "application/json; charset=utf-8");
        let body = String::from_utf8(default.body).unwrap();
        assert!(body.contains("\"source\":\"generated\""));
        assert!(body.contains("\"read_only\":true"));
        assert!(body.contains("\"max_query_bytes\":65536"));
        assert!(body.contains("\"max_rows\":3"));
        assert!(!body.contains("/tmp/"));

        let part = Config {
            part: Some(PathBuf::from("/does/not/exist")),
            database: None,
            max_rows: 300_000,
        };
        let body = handle_request("GET", "/capabilities", b"", &part).body;
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("\"source\":\"part\""));
        assert!(body.contains("\"max_rows\":250000"));

        for (method, target, body, expected) in [
            (
                "POST",
                "/capabilities",
                b"".as_slice(),
                "405 Method Not Allowed",
            ),
            ("GET", "/capabilities", b"sql".as_slice(), "400 Bad Request"),
            (
                "GET",
                "/capabilities?query=SELECT+1",
                b"".as_slice(),
                "400 Bad Request",
            ),
        ] {
            assert_eq!(
                handle_request(method, target, body, &config()).status,
                expected
            );
        }
    }

    #[test]
    fn rejects_mutations_invalid_encodings_and_conflicting_inputs() {
        for (method, target, body) in [
            ("POST", "/", &b"DROP TABLE events"[..]),
            ("GET", "/?query=%GG", &b""[..]),
            ("GET", "/?query=SELECT+1&query=SELECT+2", &b""[..]),
            ("POST", "/?query=SELECT+1", &b"SELECT 2"[..]),
            ("GET", "/unrecognized", &b""[..]),
        ] {
            assert_eq!(
                handle_request(method, target, body, &config()).status,
                "400 Bad Request"
            );
        }
    }

    #[test]
    fn response_writer_accepts_above_eight_mib() {
        let mut writer = LimitedWriter::new();
        writer.write_all(&vec![b'x'; 8 * 1024 * 1024 + 1]).unwrap();
        assert_eq!(writer.data.len(), 8 * 1024 * 1024 + 1);
        assert!(!writer.exceeded);
    }

    #[test]
    fn accepts_post_and_rejects_oversized_output() {
        let result = handle_request("POST", "/", b"SELECT 7", &config());
        assert_eq!(result.status, "200 OK");
        assert_eq!(result.body, b"7\n7\n");
        let mut writer = LimitedWriter {
            data: vec![0; MAX_RESPONSE_BYTES],
            exceeded: false,
        };
        assert!(writer.write_all(b"x").is_err());
        assert!(writer.exceeded);
        assert_eq!(writer.data.len(), MAX_RESPONSE_BYTES);
    }

    #[test]
    fn exposes_a_read_only_json_schema_for_a_frozen_part() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../rusthouse-storage/tests/fixtures/all_1_1_0");
        let config = Config {
            part: Some(path),
            database: None,
            max_rows: 1,
        };
        let response = handle_request("GET", "/schema", b"", &config);
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "application/json; charset=utf-8");
        assert_eq!(
            response.body,
            br#"{"rows":42,"columns":[{"name":"id","type":"UInt64"},{"name":"city","type":"LowCardinality(String)"}]}"#.iter().copied().chain(b"\n".iter().copied()).collect::<Vec<_>>()
        );

        for (method, path, body, status) in [
            ("POST", "/schema", &b""[..], "405 Method Not Allowed"),
            ("GET", "/schema", &b"x"[..], "400 Bad Request"),
            ("GET", "/schema?query=SELECT+1", &b""[..], "400 Bad Request"),
        ] {
            assert_eq!(handle_request(method, path, body, &config).status, status);
        }
    }

    #[test]
    fn schema_discovery_requires_a_part_and_escapes_json() {
        let missing = handle_request("GET", "/schema", b"", &config());
        assert_eq!(missing.status, "400 Bad Request");
        for url in [
            "/schema?table=events",
            "/schema?table=",
            "/schema?table=events&table=other",
            "/schema?query=SELECT+1",
            "/schema?table=%FF",
        ] {
            assert_eq!(
                handle_request("GET", url, b"", &config()).status,
                "400 Bad Request",
                "{url}"
            );
        }
        let mut output = Vec::new();
        write_json_string(&mut output, "quote\"\\newline\n\u{07}é").unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\"quote\\\"\\\\newline\\n\\u0007é\""
        );
    }

    #[test]
    fn explains_sql_without_scanning_data() {
        let get = handle_request(
            "GET",
            "/explain?query=SELECT+number+FROM+numbers%2810%29+WHERE+number%3E5",
            b"",
            &config(),
        );
        assert_eq!(get.status, "200 OK");
        assert_eq!(get.content_type, "text/plain; charset=utf-8");
        assert_eq!(
            String::from_utf8(get.body).unwrap(),
            "Logical plan (no data read):\nSource: numbers(10)\nFilter: 1 conditions across 1 OR branches\nProject: number\nLimit: 3\n"
        );

        // Planning a part query must not require --part or open data files.
        let planned = handle_request("POST", "/explain", b"SELECT id FROM part", &config());
        assert_eq!(planned.status, "200 OK");
        assert!(
            String::from_utf8(planned.body)
                .unwrap()
                .contains("Source: table part")
        );

        let executed = handle_request("POST", "/", b"SELECT id FROM part", &config());
        assert_eq!(executed.status, "400 Bad Request");
    }

    #[test]
    fn explain_endpoint_rejects_writes_and_invalid_inputs() {
        for (method, path, body, status) in [
            (
                "GET",
                "/explain?query=DROP+TABLE+t",
                &b""[..],
                "400 Bad Request",
            ),
            ("GET", "/explain", &b"SELECT 1"[..], "400 Bad Request"),
            (
                "POST",
                "/explain?query=SELECT+1",
                &b"SELECT 2"[..],
                "400 Bad Request",
            ),
            (
                "PUT",
                "/explain",
                &b"SELECT 1"[..],
                "405 Method Not Allowed",
            ),
        ] {
            assert_eq!(handle_request(method, path, body, &config()).status, status);
        }
    }

    #[test]
    fn safely_decodes_utf8_queries() {
        assert_eq!(decode_url("SELECT+1"), Ok("SELECT 1".into()));
        assert_eq!(decode_url("%FF"), Err("SQL query must be valid UTF-8"));
        assert_eq!(decode_url("%2"), Err("invalid percent encoding"));
    }
    #[test]
    fn returns_bounded_ndjson_for_get_and_post() {
        let response = handle_request(
            "GET",
            "/?query=SELECT+number+FROM+numbers%2810%29&format=jsonl",
            b"",
            &config(),
        );
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "application/x-ndjson; charset=utf-8");
        assert_eq!(
            response.body,
            b"{\"number\":\"0\"}\n{\"number\":\"1\"}\n{\"number\":\"2\"}\n"
        );

        let response = handle_request(
            "POST",
            "/?format=jsonl",
            b"SELECT count(*) FROM numbers(0)",
            &config(),
        );
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.body, b"{\"count\":\"0\"}\n");

        let response = handle_request("POST", "/?format=tsv", b"SELECT 7", &config());
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.body, b"7\n7\n");
    }

    #[test]
    fn returns_json_arrays_for_get_and_post() {
        let response = handle_request(
            "GET",
            "/?query=SELECT+number+FROM+numbers%2810%29&format=json",
            b"",
            &config(),
        );
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "application/json; charset=utf-8");
        assert_eq!(
            response.body,
            b"[{\"number\":\"0\"},{\"number\":\"1\"},{\"number\":\"2\"}]\n"
        );

        let empty = handle_request(
            "POST",
            "/?format=json",
            b"SELECT number FROM numbers(0)",
            &config(),
        );
        assert_eq!(empty.status, "200 OK");
        assert_eq!(empty.body, b"[]\n");

        let count = handle_request(
            "POST",
            "/?format=json",
            b"SELECT count(*) FROM numbers(0)",
            &config(),
        );
        assert_eq!(count.status, "200 OK");
        assert_eq!(count.body, b"[{\"count\":\"0\"}]\n");

        let invalid = handle_request(
            "POST",
            "/?format=json",
            b"SELECT number, number FROM numbers(1)",
            &config(),
        );
        assert_eq!(invalid.status, "400 Bad Request");
    }

    #[test]
    fn returns_csv_for_get_and_post_with_row_cap() {
        let get = handle_request(
            "GET",
            "/?query=SELECT+number+FROM+numbers%2810%29&format=csv",
            b"",
            &config(),
        );
        assert_eq!(get.status, "200 OK");
        assert_eq!(get.content_type, "text/csv; charset=utf-8");
        assert_eq!(get.body, b"number\r\n0\r\n1\r\n2\r\n");

        let post = handle_request(
            "POST",
            "/?format=csv",
            b"SELECT count(*) FROM numbers(0)",
            &config(),
        );
        assert_eq!(post.status, "200 OK");
        assert_eq!(post.body, b"count\r\n0\r\n");

        let bad = handle_request("POST", "/?format=csv&format=tsv", b"SELECT 1", &config());
        assert_eq!(bad.status, "400 Bad Request");
    }

    #[test]
    fn returns_csv_without_header_for_get_post_and_empty_results() {
        let get = handle_request(
            "GET",
            "/?query=SELECT+number+FROM+numbers%2810%29&format=csv-no-header",
            b"",
            &config(),
        );
        assert_eq!(get.status, "200 OK");
        assert_eq!(get.content_type, "text/csv; charset=utf-8");
        assert_eq!(get.body, b"0\r\n1\r\n2\r\n");

        let post = handle_request(
            "POST",
            "/?format=csv-no-header",
            b"SELECT count(*) FROM numbers(0)",
            &config(),
        );
        assert_eq!(post.status, "200 OK");
        assert_eq!(post.body, b"0\r\n");

        let empty = handle_request(
            "POST",
            "/?format=csv-no-header",
            b"SELECT number FROM numbers(0)",
            &config(),
        );
        assert_eq!(empty.status, "200 OK");
        assert!(empty.body.is_empty());

        let explain = handle_request(
            "GET",
            "/explain?query=SELECT+1&format=csv-no-header",
            b"",
            &config(),
        );
        assert_eq!(explain.status, "400 Bad Request");
    }

    #[test]
    fn rejects_unsupported_query_formats_and_duplicate_column_names() {
        for url in [
            "/?query=SELECT+1&format=xml",
            "/?query=SELECT+1&format=jsonl&format=jsonl",
            "/explain?query=SELECT+1&format=jsonl",
            "/?query=SELECT+number%2Cnumber+FROM+numbers%282%29&format=jsonl",
        ] {
            assert_eq!(
                handle_request("GET", url, b"", &config()).status,
                "400 Bad Request"
            );
        }
        let config = Config {
            part: None,
            database: None,
            max_rows: 0,
        };
        let empty = handle_request("POST", "/?format=jsonl", b"SELECT 1", &config);
        assert_eq!(empty.status, "200 OK");
        assert!(empty.body.is_empty());
    }

    #[test]
    fn returns_tsv_without_column_headers_for_get_and_post() {
        let get = handle_request(
            "GET",
            "/?query=SELECT+number+FROM+numbers%2810%29&format=tsv-no-header",
            b"",
            &config(),
        );
        assert_eq!(get.status, "200 OK");
        assert_eq!(get.content_type, "text/tab-separated-values; charset=utf-8");
        assert_eq!(get.body, b"0\n1\n2\n");

        let post = handle_request(
            "POST",
            "/?format=tsv-no-header",
            b"SELECT 7 AS value",
            &config(),
        );
        assert_eq!(post.status, "200 OK");
        assert_eq!(post.body, b"7\n");

        let cap = Config {
            part: None,
            database: None,
            max_rows: 0,
        };
        let empty = handle_request("POST", "/?format=tsv-no-header", b"SELECT 1", &cap);
        assert_eq!(empty.status, "200 OK");
        assert!(empty.body.is_empty());
    }
}

#[cfg(test)]
mod json_string_bulk_perf_tests {
    use super::*;

    struct CountingWriter {
        writes: usize,
        bytes: Vec<u8>,
    }

    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn encodes_long_plain_utf8_string_with_three_writes() {
        let value = "café".repeat(4096);
        let mut output = CountingWriter {
            writes: 0,
            bytes: Vec::new(),
        };
        write_json_string(&mut output, &value).unwrap();
        assert_eq!(output.writes, 3);
        assert_eq!(output.bytes.len(), value.len() + 2);
        assert_eq!(&output.bytes[1..1 + value.len()], value.as_bytes());
    }

    #[test]
    fn preserves_all_json_escape_variants() {
        let mut output = Vec::new();
        write_json_string(&mut output, "a\"b\\\n\r\t\u{08}\u{0c}\u{0001}é").unwrap();
        assert_eq!(output, "\"a\\\"b\\\\\\n\\r\\t\\b\\f\\u0001é\"".as_bytes());
    }
}
