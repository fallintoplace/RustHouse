//! Read-only SQL discovery with the same validated, locked catalog as GET /tables.
//! Unsupported DDL never reaches the persistent write path.

use super::{
    Config, LimitedWriter, QueryFormat, Response, csv::CsvWriter, csv_no_header::WithoutHeader,
    json_array::JsonArraySink, json_compact_each_row::JsonCompactEachRowWriter, jsonl::JsonlWriter,
    markdown::MarkdownWriter, sql_values::SqlValuesWriter, tables,
};
use rusthouse_query::Ddl;
use rusthouse_storage::{acquire_database_lock, inspect_query_source, resolve_database_table};
use std::io::{self, Write};

/// SQL LIKE matches ASCII catalog identifiers. Backslash quotes the next byte.
fn like_matches(name: &str, pattern: &str) -> Result<bool, &'static str> {
    let bytes = name.as_bytes();
    let pattern = pattern.as_bytes();
    let mut states = vec![false; bytes.len() + 1];
    states[0] = true;
    let mut index = 0;
    while index < pattern.len() {
        match pattern[index] {
            b'%' => {
                for cell in 1..states.len() {
                    states[cell] |= states[cell - 1];
                }
            }
            b'_' => {
                for cell in (1..states.len()).rev() {
                    states[cell] = states[cell - 1];
                }
                states[0] = false;
            }
            mut expected => {
                if expected == b'\\' {
                    index += 1;
                    expected = *pattern
                        .get(index)
                        .ok_or("trailing LIKE escape is unsupported")?;
                }
                for cell in (1..states.len()).rev() {
                    states[cell] = states[cell - 1] && bytes[cell - 1] == expected;
                }
                states[0] = false;
            }
        }
        index += 1;
    }
    Ok(states[bytes.len()])
}

fn write_rows<W: Write>(output: &mut W, heading: &str, names: &[String]) -> io::Result<()> {
    output.write_all(heading.as_bytes())?;
    output.write_all(b"\n")?;
    for name in names {
        // Catalog identifiers are validated ASCII, without tabs or newlines.
        output.write_all(name.as_bytes())?;
        output.write_all(b"\n")?;
    }
    Ok(())
}

fn respond_with_format(format: QueryFormat, heading: &str, names: &[String]) -> Response {
    let mut output = LimitedWriter::new();
    let write: io::Result<()> = match format {
        QueryFormat::Tsv => write_rows(&mut output, heading, names),
        QueryFormat::TsvNoHeader => {
            let mut without_header = WithoutHeader::new(&mut output);
            write_rows(&mut without_header, heading, names)
        }
        QueryFormat::Csv => write_rows(&mut CsvWriter::new(&mut output), heading, names),
        QueryFormat::CsvNoHeader => {
            write_rows(&mut CsvWriter::new(WithoutHeader::new(&mut output)), heading, names)
        }
        QueryFormat::Jsonl => write_rows(&mut JsonlWriter::new(&mut output), heading, names),
        QueryFormat::JsonCompactEachRow => {
            let mut compact = JsonCompactEachRowWriter::new(&mut output);
            write_rows(&mut compact, heading, names).and_then(|()| compact.finish().map(|_| ()))
        }
        QueryFormat::Json | QueryFormat::JsonPretty => {
            let mut array = if matches!(format, QueryFormat::JsonPretty) {
                JsonArraySink::new_pretty(&mut output)
            } else {
                JsonArraySink::new(&mut output)
            };
            write_rows(&mut JsonlWriter::new(&mut array), heading, names)
                .and_then(|()| array.finish().map(|_| ()))
        }
        QueryFormat::SqlValues => {
            let mut values = SqlValuesWriter::new(&mut output);
            write_rows(&mut values, heading, names).and_then(|()| values.finish())
        }
        QueryFormat::Markdown => write_rows(&mut MarkdownWriter::new(&mut output), heading, names),
    };
    match write {
        Ok(()) => Response {
            status: "200 OK",
            content_type: format.content_type(),
            body: output.data,
        },
        Err(_) => Response::text("413 Payload Too Large", "table listing exceeds 16 MiB\n"),
    }
}

pub(super) fn show_tables(ddl: Ddl, format: QueryFormat, config: &Config) -> Response {
    if config.part.is_some() {
        return Response::text("400 Bad Request", "SHOW TABLES requires --database only\n");
    }
    let Some(database) = config.database.as_deref() else {
        return Response::text("400 Bad Request", "SHOW TABLES requires --database\n");
    };
    let _read_lock = match acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Response::text("409 Conflict", &format!("{error}\n")),
    };
    let mut names = match tables::table_names(database) {
        Ok(names) => names,
        Err(message) => return Response::text("400 Bad Request", &message),
    };
    let negated = matches!(&ddl, Ddl::ShowTablesNotLike { .. });
    match ddl {
        Ddl::ShowTables => {}
        Ddl::ShowTablesLike { pattern } | Ddl::ShowTablesNotLike { pattern } => {
            let mut filtered = Vec::new();
            for name in names {
                let matched = match like_matches(&name, &pattern) {
                    Ok(matches) => matches,
                    Err(message) => return Response::text("400 Bad Request", message),
                };
                if matched != negated {
                    filtered.push(name);
                }
            }
            names = filtered;
        }
        _ => return Response::text("400 Bad Request", "unsupported read-only SHOW query\n"),
    }
    names.truncate(config.max_rows.min(250_000));
    respond_with_format(format, "name", &names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_matches_full_names_with_escaped_wildcards() {
        assert!(like_matches("events", "e%").unwrap());
        assert!(like_matches("events", "e_ent_").unwrap());
        assert!(!like_matches("events", "E%").unwrap());
        assert!(!like_matches("events", "%log%").unwrap());
        assert!(like_matches("a_b", r"a\_b").unwrap());
        assert!(!like_matches("a_b", r"a\%b").unwrap());
        assert!(like_matches("anything", "%").unwrap());
        assert!(like_matches("", "").unwrap());
        assert!(like_matches("", "%").unwrap());
        assert!(like_matches("events", r"e\").is_err());
    }

    #[test]
    fn formats_rows_without_escaping_valid_catalog_identifiers() {
        let names = vec!["events".into(), "users".into()];
        assert_eq!(
            respond_with_format(QueryFormat::Tsv, "name", &names).body,
            b"name\nevents\nusers\n"
        );
        assert_eq!(
            respond_with_format(QueryFormat::TsvNoHeader, "name", &names).body,
            b"events\nusers\n"
        );
        assert_eq!(
            respond_with_format(QueryFormat::Jsonl, "name", &names).body,
            b"{\"name\":\"events\"}\n{\"name\":\"users\"}\n"
        );
    }
}

fn write_description_rows<W: Write>(
    output: &mut W,
    columns: &[rusthouse_storage::Column],
    limit: usize,
) -> io::Result<()> {
    // ClickHouse's default DESCRIBE schema has seven columns. Unsupported
    // default expressions, codecs, comments, and TTLs are blank because
    // the local RustHouse catalog cannot represent them.
    output.write_all(b"name\ttype\tdefault_type\tdefault_expression\tcomment\tcodec_expression\tttl_expression\n")?;
    for column in columns.iter().take(limit) {
        if column.name.len() > 128
            || column.data_type.len() > 256
            || [column.name.as_str(), column.data_type.as_str()]
                .iter()
                .any(|cell| cell.bytes().any(|ch| ch == b'\t' || ch == b'\r' || ch == b'\n' || ch == 0))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid table description field",
            ));
        }
        output.write_all(column.name.as_bytes())?;
        output.write_all(b"\t")?;
        output.write_all(column.data_type.as_bytes())?;
        output.write_all(b"\t\t\t\t\t\n")?;
    }
    Ok(())
}

/// DESCRIBE never reads row data, and it shares the same lock and catalog
/// resolution used by SELECT to avoid returning stale or orphaned metadata.
pub(super) fn describe_table(name: &str, format: QueryFormat, config: &Config) -> Response {
    if config.part.is_some() {
        return Response::text("400 Bad Request", "DESCRIBE TABLE requires --database only\n");
    }
    let Some(database) = config.database.as_deref() else {
        return Response::text("400 Bad Request", "DESCRIBE TABLE requires --database\n");
    };
    let _lock = match acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Response::text("409 Conflict", &format!("{error}\n")),
    };
    let part = match resolve_database_table(database, name) {
        Ok(path) => path,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    let metadata = match inspect_query_source(&part) {
        Ok(metadata) => metadata,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    // The metadata column limit is bounded by storage, and we additionally
    // bound materialization to the existing SQL output limits.
    let mut tsv = Vec::new();
    if let Err(error) = write_description_rows(&mut tsv, &metadata.columns, config.max_rows.min(250_000)) {
        return Response::text("400 Bad Request", &format!("{error}\n"));
    }
    let mut output = LimitedWriter::new();
    let result: io::Result<()> = match format {
        QueryFormat::Tsv => output.write_all(&tsv),
        QueryFormat::TsvNoHeader => {
            let mut sink = WithoutHeader::new(&mut output);
            sink.write_all(&tsv)
        }
        QueryFormat::Csv => CsvWriter::new(&mut output).write_all(&tsv),
        QueryFormat::CsvNoHeader => {
            CsvWriter::new(WithoutHeader::new(&mut output)).write_all(&tsv)
        }
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
    match result {
        Ok(()) => Response {
            status: "200 OK",
            content_type: format.content_type(),
            body: output.data,
        },
        Err(_) => Response::text("413 Payload Too Large", "table description exceeds 16 MiB\n"),
    }
}

#[cfg(test)]
mod describe_tests {
    use super::*;

    #[test]
    fn description_has_clickhouse_seven_column_header_and_empty_defaults() {
        let columns = [
            rusthouse_storage::Column { name: "id".into(), data_type: "UInt64".into() },
            rusthouse_storage::Column { name: "email".into(), data_type: "String".into() },
        ];
        let mut data = Vec::new();
        write_description_rows(&mut data, &columns, 2).unwrap();
        assert_eq!(
            String::from_utf8(data).unwrap(),
            "name\ttype\tdefault_type\tdefault_expression\tcomment\tcodec_expression\tttl_expression\nid\tUInt64\t\t\t\t\t\nemail\tString\t\t\t\t\t\n"
        );
        let mut limited = Vec::new();
        write_description_rows(&mut limited, &columns, 1).unwrap();
        assert!(!String::from_utf8(limited).unwrap().contains("email"));
    }

    #[test]
    fn malformed_description_fields_are_rejected() {
        let bad = [rusthouse_storage::Column {
            name: "name\twrong".into(),
            data_type: "String".into(),
        }];
        assert!(write_description_rows(&mut Vec::new(), &bad, 1).is_err());
    }
}

/// Accept SQL metadata statements with no uncontrolled SQL suffix or identifiers.
fn statement_words(sql: &str) -> Vec<&str> {
    let sql = sql.trim();
    let sql = sql.strip_suffix(';').unwrap_or(sql).trim_end();
    sql.split_ascii_whitespace().collect()
}

/// Single-database mapping for standard read-only ClickHouse clients.
/// No physical database listing or user-defined databases exist in RustHouse.
pub(super) fn show_databases(sql: &str, format: QueryFormat, config: &Config) -> Response {
    let words = statement_words(sql);
    if words.len() != 2
        || !words[0].eq_ignore_ascii_case("SHOW")
        || !words[1].eq_ignore_ascii_case("DATABASES")
    {
        return Response::text("400 Bad Request", "unsupported SHOW DATABASES syntax\n");
    }
    if config.database.is_none() || config.part.is_some() {
        return Response::text("400 Bad Request", "SHOW DATABASES requires --database only\n");
    }
    let names = if config.max_rows == 0 {
        Vec::new()
    } else {
        vec!["default".to_owned()]
    };
    respond_with_format(format, "name", &names)
}

/// Return a 0/1 table existence probe from validated catalog entries.
/// This does not follow symlinks or infer names from raw directory entries.
pub(super) fn exists_table(sql: &str, format: QueryFormat, config: &Config) -> Response {
    let words = statement_words(sql);
    if words.len() != 3
        || !words[0].eq_ignore_ascii_case("EXISTS")
        || !words[1].eq_ignore_ascii_case("TABLE")
        || !valid_sql_identifier(words[2])
    {
        return Response::text("400 Bad Request", "EXISTS TABLE requires a table identifier\n");
    }
    if config.part.is_some() {
        return Response::text("400 Bad Request", "EXISTS TABLE requires --database only\n");
    }
    let Some(database) = config.database.as_deref() else {
        return Response::text("400 Bad Request", "EXISTS TABLE requires --database\n");
    };
    let _lock = match acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Response::text("409 Conflict", &format!("{error}\n")),
    };
    let names = match tables::table_names(database) {
        Ok(names) => names,
        Err(message) => return Response::text("400 Bad Request", &message),
    };
    let exists = names.binary_search_by(|name| name.as_str().cmp(words[2])).is_ok();
    let rows = if config.max_rows == 0 {
        Vec::new()
    } else {
        vec![if exists { "1".into() } else { "0".into() }]
    };
    respond_with_format(format, "result", &rows)
}

fn valid_sql_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 128
        && matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn only_whole_single_statements_and_safe_identifiers_are_accepted() {
        assert_eq!(statement_words(" EXISTS TABLE events; "), vec!["EXISTS", "TABLE", "events"]);
        assert_eq!(statement_words("SHOW DATABASES;"), vec!["SHOW", "DATABASES"]);
        assert!(valid_sql_identifier("events_2"));
        for invalid in ["events;DROP", "other.name", "a-b", "0bad", ""] {
            assert!(!valid_sql_identifier(invalid));
        }
    }
    #[test]
    fn database_discovery_requires_configured_catalog() {
        assert_eq!(
            show_databases("SHOW DATABASES", QueryFormat::Tsv, &Config::default()).status,
            "400 Bad Request"
        );
        assert_eq!(
            show_databases(
                "SHOW DATABASES",
                QueryFormat::Tsv,
                &Config { database: Some("unused".into()), max_rows: 10, ..Config::default() }
            ).body,
            b"name\ndefault\n"
        );
        assert_eq!(
            show_databases("SHOW DATABASES; DROP TABLE x", QueryFormat::Tsv, &Config::default()).status,
            "400 Bad Request"
        );
    }
}
