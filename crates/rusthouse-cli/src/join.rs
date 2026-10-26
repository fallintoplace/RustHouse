//! Execute a bounded two-table JOIN using the existing SQL output adapters.
use crate::{csv, csv_no_header, html, json_array, jsonl, markdown, vertical};
use crate::sql::SqlOutputFormat;
use rusthouse_query::{JoinSelect, execute_join};
use std::path::{Path, PathBuf};

pub(super) fn run(
    database: &Path,
    query: &JoinSelect,
    format: SqlOutputFormat,
    output_path: Option<PathBuf>,
    time: bool,
) -> Result<(), String> {
    // Protect both immutable snapshots against concurrent DROP and RENAME.
    let _lock = if database.is_dir() {
        Some(crate::database::read_lock(database)?)
    } else {
        None
    };
    let left = crate::database::select_source(database, &query.left)?;
    let right = crate::database::select_source(database, &query.right)?;
    let mut output: Box<dyn std::io::Write> = match &output_path {
        Some(path) => Box::new(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|error| format!("cannot create {}: {error}", path.display()))?,
        ),
        None => Box::new(std::io::stdout()),
    };
    let start = std::time::Instant::now();
    let result = match format {
        SqlOutputFormat::Tsv => execute_join(query, &left, &right, &mut output),
        SqlOutputFormat::TsvNoHeader => execute_join(
            query, &left, &right, &mut csv_no_header::WithoutHeader::new(&mut output),
        ),
        SqlOutputFormat::Csv => execute_join(
            query, &left, &right, &mut csv::CsvWriter::new(&mut output),
        ),
        SqlOutputFormat::CsvNoHeader => execute_join(
            query, &left, &right,
            &mut csv::CsvWriter::new(csv_no_header::WithoutHeader::new(&mut output)),
        ),
        SqlOutputFormat::Markdown => execute_join(
            query, &left, &right, &mut markdown::MarkdownWriter::new(&mut output),
        ),
        SqlOutputFormat::Html => {
            let mut writer = html::HtmlWriter::new(&mut output);
            execute_join(query, &left, &right, &mut writer).and_then(|()| {
                writer.finish().map(|_| ()).map_err(rusthouse_query::ExecutionError::Io)
            })
        }
        SqlOutputFormat::Vertical => execute_join(
            query, &left, &right, &mut vertical::VerticalWriter::new(&mut output),
        ),
        SqlOutputFormat::Jsonl => execute_join(
            query, &left, &right, &mut jsonl::JsonlWriter::new(&mut output),
        ),
        SqlOutputFormat::Json => {
            let mut writer = json_array::JsonArrayWriter::new(&mut output);
            execute_join(query, &left, &right, &mut writer).and_then(|()| {
                writer.finish().map(|_| ()).map_err(rusthouse_query::ExecutionError::Io)
            })
        }
    };
    if time {
        eprintln!("Elapsed: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
    }
    if result.is_err() {
        drop(output);
        if let Some(path) = output_path {
            let _ = std::fs::remove_file(path);
        }
    }
    result.map_err(|error| error.to_string())
}
