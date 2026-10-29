//! Read-only SQL discovery with the same validated, locked catalog as GET /tables.
//! Unsupported DDL never reaches the persistent write path.

use super::{
    Config, LimitedWriter, QueryFormat, Response, csv::CsvWriter, csv_no_header::WithoutHeader,
    json_array::JsonArraySink, json_compact_each_row::JsonCompactEachRowWriter, jsonl::JsonlWriter,
    markdown::MarkdownWriter, sql_values::SqlValuesWriter, tables,
};
use rusthouse_query::Ddl;
use rusthouse_storage::acquire_database_lock;
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

fn write_rows<W: Write>(output: &mut W, names: &[String]) -> io::Result<()> {
    output.write_all(b"name\n")?;
    for name in names {
        // Catalog identifiers are validated ASCII, without tabs or newlines.
        output.write_all(name.as_bytes())?;
        output.write_all(b"\n")?;
    }
    Ok(())
}

fn respond_with_format(format: QueryFormat, names: &[String]) -> Response {
    let mut output = LimitedWriter::new();
    let write: io::Result<()> = match format {
        QueryFormat::Tsv => write_rows(&mut output, names),
        QueryFormat::TsvNoHeader => {
            let mut without_header = WithoutHeader::new(&mut output);
            write_rows(&mut without_header, names)
        }
        QueryFormat::Csv => write_rows(&mut CsvWriter::new(&mut output), names),
        QueryFormat::CsvNoHeader => {
            write_rows(&mut CsvWriter::new(WithoutHeader::new(&mut output)), names)
        }
        QueryFormat::Jsonl => write_rows(&mut JsonlWriter::new(&mut output), names),
        QueryFormat::JsonCompactEachRow => {
            let mut compact = JsonCompactEachRowWriter::new(&mut output);
            write_rows(&mut compact, names).and_then(|()| compact.finish().map(|_| ()))
        }
        QueryFormat::Json | QueryFormat::JsonPretty => {
            let mut array = if matches!(format, QueryFormat::JsonPretty) {
                JsonArraySink::new_pretty(&mut output)
            } else {
                JsonArraySink::new(&mut output)
            };
            write_rows(&mut JsonlWriter::new(&mut array), names)
                .and_then(|()| array.finish().map(|_| ()))
        }
        QueryFormat::SqlValues => {
            let mut values = SqlValuesWriter::new(&mut output);
            write_rows(&mut values, names).and_then(|()| values.finish())
        }
        QueryFormat::Markdown => write_rows(&mut MarkdownWriter::new(&mut output), names),
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
    match ddl {
        Ddl::ShowTables => {}
        Ddl::ShowTablesLike { pattern } | Ddl::ShowTablesNotLike { pattern } => {
            let negated = matches!(ddl, Ddl::ShowTablesNotLike { .. });
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
    respond_with_format(format, &names)
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
            respond_with_format(QueryFormat::Tsv, &names).body,
            b"name\nevents\nusers\n"
        );
        assert_eq!(
            respond_with_format(QueryFormat::TsvNoHeader, &names).body,
            b"events\nusers\n"
        );
        assert_eq!(
            respond_with_format(QueryFormat::Jsonl, &names).body,
            b"{\"name\":\"events\"}\n{\"name\":\"users\"}\n"
        );
    }
}
