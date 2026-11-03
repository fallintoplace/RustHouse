//! Execute bounded JOIN queries using the existing SQL output adapters.
use crate::sql::SqlOutputFormat;
use crate::{
    csv, csv_no_header, html, json_array, json_compact_each_row, json_seq, jsonl, markdown,
    sql_values, vertical,
};
use rusthouse_query::{
    CteMaxJoin, DerivedJoinCount, JoinCorrelatedAvg, JoinCorrelatedExists, JoinSelect, Source,
    bind_join_having_subquery, bind_join_in_subquery, execute_correlated_avg,
    execute_correlated_join, execute_cte_max_join, execute_derived_join_count, execute_join,
    execute_multi_join,
};
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
    mut run: impl FnMut(&mut dyn std::io::Write) -> Result<(), rusthouse_query::ExecutionError>,
    format: SqlOutputFormat,
    mut output: &mut dyn std::io::Write,
) -> Result<(), rusthouse_query::ExecutionError> {
    match format {
        SqlOutputFormat::Tsv => run(&mut output),
        SqlOutputFormat::TsvNoHeader => run(&mut csv_no_header::WithoutHeader::new(&mut output)),
        SqlOutputFormat::Csv => run(&mut csv::CsvWriter::new(&mut output)),
        SqlOutputFormat::CsvNoHeader => run(&mut csv::CsvWriter::new(
            csv_no_header::WithoutHeader::new(&mut output),
        )),
        SqlOutputFormat::Markdown => run(&mut markdown::MarkdownWriter::new(&mut output)),
        SqlOutputFormat::Html => {
            let mut writer = html::HtmlWriter::new(&mut output);
            finish_join_output(run(&mut writer), || writer.finish())
        }
        SqlOutputFormat::Vertical => run(&mut vertical::VerticalWriter::new(&mut output)),
        SqlOutputFormat::SqlValues => {
            let mut writer = sql_values::SqlValuesWriter::new(&mut output);
            finish_join_output(run(&mut writer), || writer.finish())
        }
        SqlOutputFormat::Jsonl => run(&mut jsonl::JsonlWriter::new(&mut output)),
        SqlOutputFormat::JsonSeq => run(&mut json_seq::JsonSeqWriter::new(&mut output)),
        SqlOutputFormat::Json => {
            let mut writer = json_array::JsonArrayWriter::new(&mut output);
            finish_join_output(run(&mut writer), || writer.finish())
        }
        SqlOutputFormat::JsonCompactEachRow => {
            let mut writer = json_compact_each_row::JsonCompactEachRowWriter::new(&mut output);
            finish_join_output(run(&mut writer), || writer.finish())
        }
        SqlOutputFormat::JsonPretty => {
            let mut writer = json_array::JsonArrayWriter::new_pretty(&mut output);
            finish_join_output(run(&mut writer), || writer.finish())
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

pub(super) fn run_cte_max(
    database: &Path,
    plan: &CteMaxJoin,
    format: SqlOutputFormat,
    output_path: Option<PathBuf>,
    time: bool,
    read_lock: Option<rusthouse_storage::DatabaseLock>,
) -> Result<(), String> {
    let _lock = if read_lock.is_some() {
        read_lock
    } else if database.is_dir() {
        Some(crate::database::read_lock(database)?)
    } else {
        None
    };
    let inner = crate::database::select_source(database, plan.inner_table())?;
    let outer = crate::database::select_source(database, plan.outer_table())?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_cte_max_join(plan, &inner, &outer, &mut writer),
        format,
        output.as_mut(),
    );
    if time {
        eprintln!(
            "Elapsed: {:.3} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    if result.is_err() {
        drop(output);
        if let Some(path) = output_path {
            let _ = std::fs::remove_file(path);
        }
    }
    result.map_err(|error| error.to_string())
}

pub(super) fn run_derived_count(
    database: &Path,
    plan: &DerivedJoinCount,
    format: SqlOutputFormat,
    output_path: Option<PathBuf>,
    time: bool,
    read_lock: Option<rusthouse_storage::DatabaseLock>,
) -> Result<(), String> {
    let _lock = if read_lock.is_some() {
        read_lock
    } else if database.is_dir() {
        Some(crate::database::read_lock(database)?)
    } else {
        None
    };
    let inner = plan.inner();
    let left = crate::database::select_source(database, &inner.left)?;
    let right = crate::database::select_source(database, &inner.right)?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_derived_join_count(plan, &left, &right, &mut writer),
        format,
        output.as_mut(),
    );
    if time {
        eprintln!(
            "Elapsed: {:.3} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    if result.is_err() {
        drop(output);
        if let Some(path) = output_path {
            let _ = std::fs::remove_file(path);
        }
    }
    result.map_err(|error| error.to_string())
}

pub(super) fn run_correlated(
    database: &Path,
    plan: &JoinCorrelatedExists,
    format: SqlOutputFormat,
    output_path: Option<PathBuf>,
    time: bool,
    read_lock: Option<rusthouse_storage::DatabaseLock>,
) -> Result<(), String> {
    let _lock = if read_lock.is_some() {
        read_lock
    } else if database.is_dir() {
        Some(crate::database::read_lock(database)?)
    } else {
        None
    };
    let query = plan.outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let paths = tables
        .map(|table| crate::database::select_source(database, table))
        .collect::<Result<Vec<_>, _>>()?;
    let inner_paths = plan
        .filters
        .iter()
        .map(|filter| crate::database::select_source(database, &filter.table))
        .collect::<Result<Vec<_>, _>>()?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_correlated_join(plan, &paths, &inner_paths, &mut writer),
        format,
        output.as_mut(),
    );
    if time {
        eprintln!(
            "Elapsed: {:.3} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    if result.is_err() {
        drop(output);
        if let Some(path) = output_path {
            let _ = std::fs::remove_file(path);
        }
    }
    result.map_err(|error| error.to_string())
}

pub(super) fn run_correlated_avg(
    database: &Path,
    plan: &JoinCorrelatedAvg,
    format: SqlOutputFormat,
    output_path: Option<PathBuf>,
    time: bool,
    read_lock: Option<rusthouse_storage::DatabaseLock>,
) -> Result<(), String> {
    let _lock = if read_lock.is_some() {
        read_lock
    } else if database.is_dir() {
        Some(crate::database::read_lock(database)?)
    } else {
        None
    };
    let query = plan.outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let paths = tables
        .map(|table| crate::database::select_source(database, table))
        .collect::<Result<Vec<_>, _>>()?;
    let inner = crate::database::select_source(database, &plan.table)?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_correlated_avg(plan, &paths, &inner, &mut writer),
        format,
        output.as_mut(),
    );
    if time {
        eprintln!(
            "Elapsed: {:.3} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    if result.is_err() {
        drop(output);
        if let Some(path) = output_path {
            let _ = std::fs::remove_file(path);
        }
    }
    result.map_err(|error| error.to_string())
}

pub(super) fn run(
    database: &Path,
    query: &JoinSelect,
    format: SqlOutputFormat,
    output_path: Option<PathBuf>,
    time: bool,
    read_lock: Option<rusthouse_storage::DatabaseLock>,
) -> Result<(), String> {
    // Protect both immutable snapshots against concurrent DROP and RENAME.
    let _lock = if read_lock.is_some() {
        read_lock
    } else if database.is_dir() {
        Some(crate::database::read_lock(database)?)
    } else {
        None
    };
    let mut paths = Vec::with_capacity(query.steps.len() + 2);
    paths.push(crate::database::select_source(database, &query.left)?);
    paths.push(crate::database::select_source(database, &query.right)?);
    for step in &query.steps {
        paths.push(crate::database::select_source(database, &step.table)?);
    }
    let query_started_at = std::time::Instant::now();
    let mut bound = query.clone();
    if let Some(inner) = bound
        .grouped_sum_having
        .as_ref()
        .and_then(|having| having.subquery.as_ref())
    {
        let tables = [&inner.left, &inner.right]
            .into_iter()
            .chain(inner.steps.iter().map(|step| &step.table));
        let inner_paths = tables
            .map(|table| crate::database::select_source(database, table))
            .collect::<Result<Vec<_>, _>>()?;
        bind_join_having_subquery(&mut bound, &inner_paths).map_err(|error| error.to_string())?;
    }
    if let Some(subquery) = &bound.in_subquery {
        let Some(Source::Table(table)) = &subquery.inner.source else {
            return Err("JOIN IN subquery requires a stored table".into());
        };
        let inner_path = crate::database::select_source(database, table)?;
        bind_join_in_subquery(&mut bound, &inner_path, &paths)
            .map_err(|error| error.to_string())?;
    }
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_selected_join(&bound, &paths, &mut writer),
        format,
        output.as_mut(),
    );
    if time {
        eprintln!(
            "Elapsed: {:.3} ms",
            query_started_at.elapsed().as_secs_f64() * 1000.0
        );
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
