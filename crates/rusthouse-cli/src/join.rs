//! Execute bounded JOIN queries using the existing SQL output adapters.
use crate::sql::SqlOutputFormat;
use crate::{
    csv, csv_no_header, html, json_array, json_compact_each_row, json_seq, jsonl, markdown,
    sql_values, vertical,
};
use rusthouse_query::{JoinSelect, execute_join, execute_multi_join};
use std::path::{Path, PathBuf};

fn execute_selected_join<W: std::io::Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    output: &mut W,
) -> Result<(), rusthouse_query::ExecutionError> {
    if paths.len() == 2 {
        execute_join(query, &paths[0], &paths[1], output)
    } else {
        execute_multi_join(query, paths, output)
    }
}

fn finish_join_output<T>(
    result: Result<(), rusthouse_query::ExecutionError>,
    finish: impl FnOnce() -> std::io::Result<T>,
) -> Result<(), rusthouse_query::ExecutionError> {
    result.and_then(|()| {
        finish()
            .map(|_| ())
            .map_err(rusthouse_query::ExecutionError::Io)
    })
}

fn execute_formatted_join(
    query: &JoinSelect,
    paths: &[PathBuf],
    format: SqlOutputFormat,
    mut output: &mut dyn std::io::Write,
) -> Result<(), rusthouse_query::ExecutionError> {
    match format {
        SqlOutputFormat::Tsv => execute_selected_join(query, paths, &mut output),
        SqlOutputFormat::TsvNoHeader => execute_selected_join(
            query,
            paths,
            &mut csv_no_header::WithoutHeader::new(&mut output),
        ),
        SqlOutputFormat::Csv => {
            execute_selected_join(query, paths, &mut csv::CsvWriter::new(&mut output))
        }
        SqlOutputFormat::CsvNoHeader => execute_selected_join(
            query,
            paths,
            &mut csv::CsvWriter::new(csv_no_header::WithoutHeader::new(&mut output)),
        ),
        SqlOutputFormat::Markdown => execute_selected_join(
            query,
            paths,
            &mut markdown::MarkdownWriter::new(&mut output),
        ),
        SqlOutputFormat::Html => {
            let mut writer = html::HtmlWriter::new(&mut output);
            finish_join_output(execute_selected_join(query, paths, &mut writer), || {
                writer.finish()
            })
        }
        SqlOutputFormat::Vertical => execute_selected_join(
            query,
            paths,
            &mut vertical::VerticalWriter::new(&mut output),
        ),
        SqlOutputFormat::SqlValues => {
            let mut writer = sql_values::SqlValuesWriter::new(&mut output);
            finish_join_output(execute_selected_join(query, paths, &mut writer), || {
                writer.finish()
            })
        }
        SqlOutputFormat::Jsonl => {
            execute_selected_join(query, paths, &mut jsonl::JsonlWriter::new(&mut output))
        }
        SqlOutputFormat::JsonSeq => {
            execute_selected_join(query, paths, &mut json_seq::JsonSeqWriter::new(&mut output))
        }
        SqlOutputFormat::Json => {
            let mut writer = json_array::JsonArrayWriter::new(&mut output);
            finish_join_output(execute_selected_join(query, paths, &mut writer), || {
                writer.finish()
            })
        }
        SqlOutputFormat::JsonCompactEachRow => {
            let mut writer = json_compact_each_row::JsonCompactEachRowWriter::new(&mut output);
            finish_join_output(execute_selected_join(query, paths, &mut writer), || {
                writer.finish()
            })
        }
        SqlOutputFormat::JsonPretty => {
            let mut writer = json_array::JsonArrayWriter::new_pretty(&mut output);
            finish_join_output(execute_selected_join(query, paths, &mut writer), || {
                writer.finish()
            })
        }
    }
}

fn open_output(path: Option<&Path>) -> Result<Box<dyn std::io::Write>, String> {
    let output: Box<dyn std::io::Write> = match path {
        Some(path) => Box::new(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|error| format!("cannot create {}: {error}", path.display()))?,
        ),
        None => Box::new(std::io::stdout()),
    };
    Ok(output)
}

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
    let mut paths = vec![
        crate::database::select_source(database, &query.left)?,
        crate::database::select_source(database, &query.right)?,
    ];
    for step in &query.steps {
        paths.push(crate::database::select_source(database, &step.table)?);
    }
    let mut output = open_output(output_path.as_deref())?;
    let start = std::time::Instant::now();
    let result = execute_formatted_join(query, &paths, format, output.as_mut());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_creation_does_not_overwrite_existing_files() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-join-output-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"existing").unwrap();

        assert!(open_output(Some(&path)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");

        std::fs::remove_file(path).unwrap();
    }
}
