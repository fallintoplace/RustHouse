//! Read-only DESCRIBE of persistent tables using validated snapshot schemas.
use super::*;
use rusthouse_storage::acquire_database_lock;

pub(super) fn handle(config: &Config, format: QueryFormat, table: &str) -> Response {
    if config.part.is_some() {
        return Response::text("400 Bad Request", "DESCRIBE requires --database\n");
    }
    let Some(database) = config.database.as_deref() else {
        return Response::text("400 Bad Request", "DESCRIBE requires --database\n");
    };
    let _lock = match acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Response::text("409 Conflict", &format!("{error}\n")),
    };
    let path = match resolve_database_table(database, table) {
        Ok(path) => path,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    let schema = match inspect_query_source(&path) {
        Ok(schema) => schema,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    let emit = |output: &mut dyn Write| -> io::Result<()> {
        output.write_all(b"name\ttype\n")?;
        for column in &schema.columns {
            output.write_all(column.name.as_bytes())?;
            output.write_all(b"\t")?;
            output.write_all(column.data_type.as_bytes())?;
            output.write_all(b"\n")?;
        }
        Ok(())
    };
    let mut writer = LimitedWriter::new();
    let result = match format {
        QueryFormat::Tsv => emit(&mut writer),
        QueryFormat::TsvNoHeader => emit(&mut WithoutHeader::new(&mut writer)),
        QueryFormat::Csv => emit(&mut CsvWriter::new(&mut writer)),
        QueryFormat::CsvNoHeader => emit(&mut CsvWriter::new(WithoutHeader::new(&mut writer))),
        QueryFormat::Jsonl => emit(&mut JsonlWriter::new(&mut writer)),
        QueryFormat::Markdown => emit(&mut MarkdownWriter::new(&mut writer)),
        QueryFormat::SqlValues => {
            let mut values = SqlValuesWriter::new(&mut writer);
            emit(&mut values).and_then(|()| values.finish())
        }
        QueryFormat::JsonCompactEachRow => {
            let mut compact = JsonCompactEachRowWriter::new(&mut writer);
            emit(&mut compact).and_then(|()| compact.finish().map(|_| ()))
        }
        QueryFormat::Json | QueryFormat::JsonPretty => {
            let mut array = if matches!(format, QueryFormat::JsonPretty) {
                JsonArraySink::new_pretty(&mut writer)
            } else {
                JsonArraySink::new(&mut writer)
            };
            emit(&mut JsonlWriter::new(&mut array)).and_then(|()| array.finish())
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
            Response::text("413 Payload Too Large", "schema result exceeds 16 MiB\n")
        }
        Err(error) => Response::text("400 Bad Request", &format!("{error}\n")),
    }
}
