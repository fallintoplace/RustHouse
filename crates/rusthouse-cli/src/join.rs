//! Execute bounded JOIN queries using the existing SQL output adapters.
use crate::sql::SqlOutputFormat;
use crate::{
    csv, csv_no_header, html, json_array, json_compact_each_row, json_seq, jsonl, markdown,
    sql_values, vertical,
};
use rusthouse_query::{
    CteCorrelatedAvg, CteMaxJoin, DerivedGroupedJoin, DerivedGroupedSelect, DerivedJoinCount,
    DerivedUnionJoin, DerivedUnionSelect, JoinCorrelatedAvg, JoinCorrelatedExists,
    JoinCorrelatedMin, JoinCorrelatedSum, JoinSelect, NestedJoinSum, Source,
    bind_join_having_subquery, bind_join_in_subquery, execute_correlated_avg_with_paths,
    execute_correlated_join, execute_correlated_min, execute_correlated_sum, execute_cte_max_join,
    execute_derived_grouped_join, execute_derived_grouped_select, execute_derived_join_count,
    execute_derived_union_join, execute_derived_union_select, execute_join, execute_multi_join,
    execute_nested_join_sum,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_CTE_RESULT_BYTES: usize = 64 * 1024 * 1024;
const MAX_CTE_RESULT_ROWS: usize = 262_144;
static NEXT_CTE_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TemporaryCtePart {
    root: PathBuf,
    path: PathBuf,
}

impl Drop for TemporaryCtePart {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn materialize_cte_part(
    output: &[u8],
    columns: &[rusthouse_query::CteOutputColumn],
) -> Result<TemporaryCtePart, String> {
    if output.len() > MAX_CTE_RESULT_BYTES {
        return Err("grouped CTE result exceeds 64 MiB".into());
    }
    let output = std::str::from_utf8(output)
        .map_err(|_| "grouped CTE output is not valid UTF-8".to_owned())?;
    let mut lines = output.split_terminator('\n');
    let header = lines
        .next()
        .ok_or_else(|| "grouped CTE output has no header".to_owned())?;
    if header
        .split('\t')
        .ne(columns.iter().map(|column| column.name.as_str()))
    {
        return Err("grouped CTE output columns differ from its plan".into());
    }
    let mut rows = Vec::new();
    for line in lines {
        if rows.len() == MAX_CTE_RESULT_ROWS {
            return Err("grouped CTE result exceeds 262144 rows".into());
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != columns.len() {
            return Err("grouped CTE output has an invalid row width".into());
        }
        let mut row = Vec::with_capacity(fields.len());
        for field in fields {
            if field == r"\N" {
                row.push(None);
                continue;
            }
            let decoded = crate::tsv_escape::decode_tsv_field(field.as_bytes())
                .map_err(|error| error.to_string())?;
            row.push(Some(String::from_utf8(decoded).map_err(|_| {
                "grouped CTE value is not valid UTF-8".to_owned()
            })?));
        }
        rows.push(row);
    }
    let mut root = None;
    for _ in 0..16 {
        let id = NEXT_CTE_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let candidate =
            std::env::temp_dir().join(format!("rusthouse-cte-{}-{id}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => {
                root = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    let root = root.ok_or_else(|| "cannot allocate a temporary grouped CTE".to_owned())?;
    let part = root.join("part");
    let temporary = TemporaryCtePart {
        root,
        path: part.clone(),
    };
    let schema = columns
        .iter()
        .map(|column| rusthouse_storage::Column {
            name: column.name.clone(),
            data_type: column.data_type.clone(),
        })
        .collect::<Vec<_>>();
    rusthouse_storage::write_wide_part(&part, &schema, &rows).map_err(|error| error.to_string())?;
    Ok(temporary)
}

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

pub(super) fn run_cte_correlated_avg(
    database: &Path,
    plan: &CteCorrelatedAvg,
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
    let started = std::time::Instant::now();
    let grouped = plan.grouped();
    let grouped_paths = [&grouped.left, &grouped.right]
        .into_iter()
        .chain(grouped.steps.iter().map(|step| &step.table))
        .map(|table| crate::database::select_source(database, table))
        .collect::<Result<Vec<_>, _>>()?;
    let mut cte_output = Vec::new();
    execute_multi_join(grouped, &grouped_paths, &mut cte_output)
        .map_err(|error| error.to_string())?;
    let temporary = materialize_cte_part(&cte_output, plan.columns())?;

    let correlated = plan.correlated();
    let outer = correlated.outer();
    let paths = [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table))
        .map(|table| {
            if table.eq_ignore_ascii_case(plan.name()) {
                Ok(temporary.path.clone())
            } else {
                crate::database::select_source(database, table)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let inner_paths = correlated
        .inner_tables()
        .iter()
        .map(|table| {
            if table.eq_ignore_ascii_case(plan.name()) {
                Ok(temporary.path.clone())
            } else {
                crate::database::select_source(database, table)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| {
            execute_correlated_avg_with_paths(correlated, &paths, &inner_paths, &mut writer)
        },
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

pub(super) fn run_cte_multi_join(
    database: &Path,
    plan: &rusthouse_query::CteMultiJoin,
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
    let started = std::time::Instant::now();
    let mut cte_paths = HashMap::new();
    let mut temporary_parts = Vec::with_capacity(plan.sources().len());

    for source in plan.sources() {
        let query = source.query();
        let tables = [&query.left, &query.right]
            .into_iter()
            .chain(query.steps.iter().map(|step| &step.table));
        let paths = tables
            .map(|table| {
                cte_paths
                    .get(&table.to_ascii_lowercase())
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| crate::database::select_source(database, table))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut cte_output = Vec::new();
        execute_selected_join(query, &paths, &mut cte_output).map_err(|error| error.to_string())?;
        let temporary = materialize_cte_part(&cte_output, source.columns())?;
        cte_paths.insert(source.name().to_ascii_lowercase(), temporary.path.clone());
        temporary_parts.push(temporary);
    }

    let query = plan.outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let paths = tables
        .map(|table| {
            cte_paths
                .get(&table.to_ascii_lowercase())
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| crate::database::select_source(database, table))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_selected_join(query, &paths, &mut writer),
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
    drop(temporary_parts);
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

pub(super) fn run_derived_grouped(
    database: &Path,
    plan: &DerivedGroupedJoin,
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
    let inner_paths = [&inner.left, &inner.right]
        .into_iter()
        .chain(inner.steps.iter().map(|step| &step.table))
        .map(|table| crate::database::select_source(database, table))
        .collect::<Result<Vec<_>, _>>()?;
    let outer = plan.outer();
    let outer_paths = [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table))
        .map(|table| {
            if table.eq_ignore_ascii_case(plan.table()) {
                Ok(PathBuf::new())
            } else {
                crate::database::select_source(database, table)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_derived_grouped_join(plan, &inner_paths, &outer_paths, &mut writer),
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

pub(super) fn run_derived_grouped_select(
    database: &Path,
    plan: &DerivedGroupedSelect,
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
    let inner_paths = [&inner.left, &inner.right]
        .into_iter()
        .chain(inner.steps.iter().map(|step| &step.table))
        .map(|table| crate::database::select_source(database, table))
        .collect::<Result<Vec<_>, _>>()?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_derived_grouped_select(plan, &inner_paths, &mut writer),
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

pub(super) fn run_derived_union_select(
    database: &Path,
    plan: &DerivedUnionSelect,
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
    let branch_paths = plan
        .branches()
        .iter()
        .map(|branch| {
            [&branch.left, &branch.right]
                .into_iter()
                .chain(branch.steps.iter().map(|step| &step.table))
                .map(|table| crate::database::select_source(database, table))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_derived_union_select(plan, &branch_paths, &mut writer),
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

pub(super) fn run_derived_union(
    database: &Path,
    plan: &DerivedUnionJoin,
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
    let branch_paths = plan
        .branches()
        .iter()
        .map(|branch| {
            [&branch.left, &branch.right]
                .into_iter()
                .chain(branch.steps.iter().map(|step| &step.table))
                .map(|table| crate::database::select_source(database, table))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let outer = plan.outer();
    let outer_paths = [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table))
        .map(|table| {
            if table.eq_ignore_ascii_case(plan.table()) {
                Ok(PathBuf::new())
            } else {
                crate::database::select_source(database, table)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_derived_union_join(plan, &branch_paths, &outer_paths, &mut writer),
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
    let mut inner_paths = Vec::with_capacity(
        plan.filters.len()
            + plan
                .filters
                .iter()
                .filter(|filter| filter.date_filter.is_some())
                .count(),
    );
    for filter in &plan.filters {
        inner_paths.push(crate::database::select_source(database, &filter.table)?);
        if let Some(date) = &filter.date_filter {
            inner_paths.push(crate::database::select_source(database, &date.table)?);
        }
    }
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
    let inner_paths = plan
        .inner_tables()
        .iter()
        .map(|table| crate::database::select_source(database, table))
        .collect::<Result<Vec<_>, _>>()?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_correlated_avg_with_paths(plan, &paths, &inner_paths, &mut writer),
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

pub(super) fn run_nested_sum(
    database: &Path,
    plan: &NestedJoinSum,
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
    let query = plan.inner().outer();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let inner = tables
        .map(|table| crate::database::select_source(database, table))
        .collect::<Result<Vec<_>, _>>()?;
    let sum = crate::database::select_source(database, &plan.inner().table)?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_nested_join_sum(plan, &paths, &inner, &sum, &mut writer),
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

pub(super) fn run_correlated_sum(
    database: &Path,
    plan: &JoinCorrelatedSum,
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
        |mut writer| execute_correlated_sum(plan, &paths, &inner, &mut writer),
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

pub(super) fn run_correlated_min(
    database: &Path,
    plan: &JoinCorrelatedMin,
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
    let query = plan.inner();
    let tables = [&query.left, &query.right]
        .into_iter()
        .chain(query.steps.iter().map(|step| &step.table));
    let inner = tables
        .map(|table| crate::database::select_source(database, table))
        .collect::<Result<Vec<_>, _>>()?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| execute_correlated_min(plan, &paths, &inner, &mut writer),
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

pub(super) fn run_substring_antijoin(
    database: &Path,
    plan: &rusthouse_query::SubstringAntiJoin,
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
    let source = crate::database::select_source(database, &plan.table)?;
    let lookup = crate::database::select_source(database, &plan.lookup_table)?;
    let started = std::time::Instant::now();
    let mut output = open_output(output_path.as_deref())?;
    let result = execute_formatted_join(
        |mut writer| {
            rusthouse_query::execute_substring_antijoin(plan, &source, &lookup, &mut writer)
        },
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
