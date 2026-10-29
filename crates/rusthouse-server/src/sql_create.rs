//! Read-only logical SHOW CREATE TABLE introspection.
//! A RustHouse catalog lacks MergeTree engine, partitioning and sorting
//! metadata; returning invented physical DDL would be unsafe.
use super::{
    Config, LimitedWriter, QueryFormat, Response, csv::CsvWriter,
    csv_no_header::WithoutHeader, json_array::JsonArraySink,
    json_compact_each_row::JsonCompactEachRowWriter, jsonl::JsonlWriter,
    markdown::MarkdownWriter, sql_values::SqlValuesWriter,
};
use rusthouse_storage::{Column, acquire_database_lock, inspect_query_source, resolve_database_table};
use std::io::{self, Write};

fn identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    !name.is_empty()
        && name.len() <= 128
        && matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn logical_ddl(name: &str, columns: &[Column]) -> Result<String, &'static str> {
    if !identifier(name) || columns.is_empty() || columns.len() > 64 {
        return Err("invalid SHOW CREATE TABLE metadata");
    }
    let mut ddl = format!("CREATE TABLE {name} (");
    for (index, column) in columns.iter().enumerate() {
        if !identifier(&column.name)
            || column.data_type.len() > 256
            || column.data_type.bytes().any(|ch| ch == b'\n' || ch == b'\t' || ch == b'\r' || ch == 0)
        {
            return Err("invalid SHOW CREATE TABLE column metadata");
        }
        if index > 0 {
            ddl.push_str(", ");
        }
        ddl.push_str(&column.name);
        ddl.push(' ');
        ddl.push_str(&column.data_type);
    }
    ddl.push(')');
    Ok(ddl)
}

pub(super) fn show_create_table(name: &str, format: QueryFormat, config: &Config) -> Response {
    if config.part.is_some() {
        return Response::text("400 Bad Request", "SHOW CREATE TABLE requires --database only\n");
    }
    let Some(database) = config.database.as_deref() else {
        return Response::text("400 Bad Request", "SHOW CREATE TABLE requires --database\n");
    };
    let _lock = match acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Response::text("409 Conflict", &format!("{error}\n")),
    };
    let part = match resolve_database_table(database, name) {
        Ok(part) => part,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    let metadata = match inspect_query_source(&part) {
        Ok(metadata) => metadata,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    let ddl = match logical_ddl(name, &metadata.columns) {
        Ok(ddl) => ddl,
        Err(message) => return Response::text("400 Bad Request", message),
    };
    let mut tsv = b"statement\n".to_vec();
    if config.max_rows > 0 {
        tsv.extend_from_slice(ddl.as_bytes());
        tsv.push(b'\n');
    }

    let mut output = LimitedWriter::new();
    let write: io::Result<()> = match format {
        QueryFormat::Tsv => output.write_all(&tsv),
        QueryFormat::TsvNoHeader => WithoutHeader::new(&mut output).write_all(&tsv),
        QueryFormat::Csv => CsvWriter::new(&mut output).write_all(&tsv),
        QueryFormat::CsvNoHeader => CsvWriter::new(WithoutHeader::new(&mut output)).write_all(&tsv),
        QueryFormat::Jsonl => JsonlWriter::new(&mut output).write_all(&tsv),
        QueryFormat::JsonCompactEachRow => {
            let mut sink = JsonCompactEachRowWriter::new(&mut output);
            sink.write_all(&tsv).and_then(|()| sink.finish().map(|_| ()))
        }
        QueryFormat::Json | QueryFormat::JsonPretty => {
            let mut array = if matches!(format, QueryFormat::JsonPretty) {
                JsonArraySink::new_pretty(&mut output)
            } else {
                JsonArraySink::new(&mut output)
            };
            JsonlWriter::new(&mut array)
                .write_all(&tsv)
                .and_then(|()| array.finish().map(|_| ()))
        }
        QueryFormat::SqlValues => {
            let mut sink = SqlValuesWriter::new(&mut output);
            sink.write_all(&tsv).and_then(|()| sink.finish())
        }
        QueryFormat::Markdown => MarkdownWriter::new(&mut output).write_all(&tsv),
    };
    match write {
        Ok(()) => Response {
            status: "200 OK",
            content_type: format.content_type(),
            body: output.data,
        },
        Err(_) => Response::text("413 Payload Too Large", "SHOW CREATE TABLE output exceeds 16 MiB\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_schema_excludes_unavailable_clickhouse_engine_properties() {
        let columns = [
            Column { name: "id".into(), data_type: "UInt64".into() },
            Column { name: "label".into(), data_type: "Nullable(String)".into() },
        ];
        let ddl = logical_ddl("events", &columns).unwrap();
        assert_eq!(ddl, "CREATE TABLE events (id UInt64, label Nullable(String))");
        assert!(!ddl.contains("ENGINE"));
        assert!(!ddl.contains("ORDER BY"));
    }

    #[test]
    fn rejects_malicious_or_invalid_schema_names() {
        let columns = [Column { name: "id".into(), data_type: "UInt64".into() }];
        assert!(logical_ddl("events; DROP TABLE x", &columns).is_err());
        let columns = [Column { name: "id\nattack".into(), data_type: "String".into() }];
        assert!(logical_ddl("events", &columns).is_err());
    }
}
