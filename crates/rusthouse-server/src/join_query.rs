//! Bounded read-only JOIN queries through the SQL HTTP interface.

use super::*;
use rusthouse_query::{ExecutionError, JoinSelect, execute_join, execute_multi_join, explain_join};

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
    query.limit = Some(query.limit.map_or(join_row_cap, |limit| limit.min(join_row_cap)));
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
    let tables = std::iter::once(&query.left)
        .chain(std::iter::once(&query.right))
        .chain(query.steps.iter().map(|step| &step.table));
    let mut paths = Vec::with_capacity(query.steps.len() + 2);
    for table in tables {
        match resolve_database_table(database, table) {
            Ok(path) => paths.push(path),
            Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
        }
    }
    let mut writer = LimitedWriter::new();
    let run = |mut output: &mut dyn Write| -> Result<(), ExecutionError> {
        if paths.len() == 2 {
            execute_join(query, &paths[0], &paths[1], &mut output)
        } else {
            execute_multi_join(query, &paths, &mut output)
        }
    };
    let result = match format {
        QueryFormat::Tsv | QueryFormat::TsvNoHeader => run(&mut writer),
        QueryFormat::Csv => run(&mut CsvWriter::new(&mut writer)),
        QueryFormat::CsvNoHeader => {
            run(&mut CsvWriter::new(WithoutHeader::new(&mut writer)))
        }
        QueryFormat::Jsonl => run(&mut JsonlWriter::new(&mut writer)),
        QueryFormat::Markdown => run(&mut MarkdownWriter::new(&mut writer)),
        QueryFormat::SqlValues => {
            let mut values = SqlValuesWriter::new(&mut writer);
            run(&mut values)
                .and_then(|()| values.finish().map_err(ExecutionError::Io))
        }
        QueryFormat::JsonCompactEachRow => {
            let mut compact = JsonCompactEachRowWriter::new(&mut writer);
            run(&mut compact)
                .and_then(|()| compact.finish().map(|_| ()).map_err(ExecutionError::Io))
        }
        QueryFormat::Json | QueryFormat::JsonPretty => {
            let mut array = if matches!(format, QueryFormat::JsonPretty) {
                JsonArraySink::new_pretty(&mut writer)
            } else {
                JsonArraySink::new(&mut writer)
            };
            let result = run(&mut JsonlWriter::new(&mut array));
            result.and_then(|()| array.finish().map(|_| ()).map_err(ExecutionError::Io))
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
                QueryFormat::Json | QueryFormat::JsonPretty => {
                    "application/json; charset=utf-8"
                }
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
