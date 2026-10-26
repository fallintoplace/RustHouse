//! Minimal read-only SQL HTTP interface, bound to loopback by the binary.
//! This is intentionally not a full database wire protocol.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::inspect_query_source;
use std::io::{self, Write};
use std::path::PathBuf;

pub const MAX_QUERY_BYTES: usize = 4096;
pub const MAX_HEADER_BYTES: usize = 8192;
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CONNECTIONS: usize = 16;

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub part: Option<PathBuf>,
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
}

impl LimitedWriter {
    fn new() -> Self {
        Self { data: Vec::new() }
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let new_size =
            self.data.len().checked_add(bytes.len()).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "response size overflow")
            })?;
        if new_size > MAX_RESPONSE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SQL response exceeds 8 MiB",
            ));
        }
        self.data.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn decode_url(value: &str) -> Result<String, &'static str> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index];
        if ch == b'%' {
            if index + 2 >= bytes.len() {
                return Err("invalid percent encoding");
            }
            let hex = |v: u8| (v as char).to_digit(16).map(|digit| digit as u8);
            let hi = hex(bytes[index + 1]).ok_or("invalid percent encoding")?;
            let lo = hex(bytes[index + 2]).ok_or("invalid percent encoding")?;
            decoded.push((hi << 4) | lo);
            index += 3;
        } else {
            decoded.push(if ch == b'+' { b' ' } else { ch });
            index += 1;
        }
        if decoded.len() > MAX_QUERY_BYTES {
            return Err("SQL statement exceeds 4096 bytes");
        }
    }
    String::from_utf8(decoded).map_err(|_| "SQL query must be valid UTF-8")
}

fn write_json_string<W: Write>(writer: &mut W, value: &str) -> io::Result<()> {
    writer.write_all(b"\"")?;
    for ch in value.chars() {
        match ch {
            '"' => writer.write_all(b"\\\"")?,
            '\\' => writer.write_all(b"\\\\")?,
            '\n' => writer.write_all(b"\\n")?,
            '\r' => writer.write_all(b"\\r")?,
            '\t' => writer.write_all(b"\\t")?,
            '\u{08}' => writer.write_all(b"\\b")?,
            '\u{0c}' => writer.write_all(b"\\f")?,
            ch if ch <= '\u{1f}' => write!(writer, "\\u{:04x}", ch as u32)?,
            ch => writer.write_all(ch.encode_utf8(&mut [0; 4]).as_bytes())?,
        }
    }
    writer.write_all(b"\"")
}

fn schema_response(config: &Config) -> Response {
    let Some(part) = config.part.as_deref() else {
        return Response::text("400 Bad Request", "no --part source configured\n");
    };
    let metadata = match inspect_query_source(part) {
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
        Err(_) => Response::text("413 Payload Too Large", "schema response exceeds 8 MiB\n"),
    }
}

fn query_parameters(target: &str) -> Result<(String, Option<String>), &'static str> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" {
        return Err("unknown SQL endpoint");
    }
    let mut statement = None;
    for parameter in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = parameter
            .split_once('=')
            .ok_or("invalid URL query parameter")?;
        if key != "query" {
            return Err("unsupported URL query parameter");
        }
        if statement.is_some() {
            return Err("duplicate query parameter");
        }
        statement = Some(decode_url(value)?);
    }
    Ok((path.to_owned(), statement))
}

/// Runs only the subset accepted by the existing SELECT parser. DDL and
/// writes cannot pass this interface. All responses are bounded in memory.
pub fn handle_request(method: &str, target: &str, body: &[u8], config: &Config) -> Response {
    if method == "GET" && target == "/ping" {
        return Response::text("200 OK", "Ok.\n");
    }
    if target == "/schema" {
        if method != "GET" {
            return Response::text("405 Method Not Allowed", "schema endpoint requires GET\n");
        }
        if !body.is_empty() {
            return Response::text("400 Bad Request", "GET must not contain a request body\n");
        }
        return schema_response(config);
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
    let (_, query) = match query_parameters(target) {
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
    let cap = config.max_rows.min(100_000);
    select.limit = Some(select.limit.map_or(cap, |limit| limit.min(cap)));
    let mut writer = LimitedWriter::new();
    match execute(&select, config.part.as_deref(), &mut writer) {
        Ok(()) => Response {
            status: "200 OK",
            content_type: "text/tab-separated-values; charset=utf-8",
            body: writer.data,
        },
        Err(error) => Response::text("400 Bad Request", &format!("{error}\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            part: None,
            max_rows: 3,
        }
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
    fn accepts_post_and_rejects_oversized_output() {
        let result = handle_request("POST", "/", b"SELECT 7", &config());
        assert_eq!(result.status, "200 OK");
        assert_eq!(result.body, b"7\n7\n");
        let mut writer = LimitedWriter {
            data: vec![0; MAX_RESPONSE_BYTES],
        };
        assert!(writer.write_all(b"x").is_err());
        assert_eq!(writer.data.len(), MAX_RESPONSE_BYTES);
    }

    #[test]
    fn exposes_a_read_only_json_schema_for_a_frozen_part() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../rusthouse-storage/tests/fixtures/all_1_1_0");
        let config = Config {
            part: Some(path),
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
        let mut output = Vec::new();
        write_json_string(&mut output, "quote\"\\newline\n\u{07}é").unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\"quote\\\"\\\\newline\\n\\u0007é\""
        );
    }

    #[test]
    fn safely_decodes_utf8_queries() {
        assert_eq!(decode_url("SELECT+1"), Ok("SELECT 1".into()));
        assert_eq!(decode_url("%FF"), Err("SQL query must be valid UTF-8"));
        assert_eq!(decode_url("%2"), Err("invalid percent encoding"));
    }
}
