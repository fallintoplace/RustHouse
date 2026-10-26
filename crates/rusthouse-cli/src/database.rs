//! Single-writer local database mode.
//! Tables use existing .schema metadata and immutable, atomically published wide parts.
use crate::catalog;
use rusthouse_query::{Ddl, Insert};
use rusthouse_storage::{
    Column, inspect_query_source, query_parts, verify_wide_part, write_wide_part,
};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_STAGE: AtomicUsize = AtomicUsize::new(0);

struct WriteLock {
    path: PathBuf,
    _file: fs::File,
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn lock(database: &Path) -> Result<WriteLock, String> {
    fs::create_dir_all(database)
        .map_err(|error| format!("cannot create database {}: {error}", database.display()))?;
    let path = database.join(".rusthouse-write.lock");
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("cannot acquire database write lock {}: {error}", path.display()))?;
    Ok(WriteLock { path, _file: file })
}

fn data_path(database: &Path, name: &str) -> PathBuf {
    database.join(format!("{name}.parts"))
}

fn columns(database: &Path, name: &str) -> Result<Vec<Column>, String> {
    let schema = catalog::read_schema(database, name)?;
    Ok(schema.into_iter().map(|col| Column {
        name: col.name,
        data_type: col.data_type,
    }).collect())
}

fn validated_parts(database: &Path, name: &str, cols: &[Column]) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let path = data_path(database, name);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot inspect table data {name}: {error}"))?;
    if !metadata.file_type().is_dir() {
        return Err(format!("table data is not a real directory: {name}"));
    }
    let parts = query_parts(&path).map_err(|error| error.to_string())?;
    let source = inspect_query_source(&path).map_err(|error| error.to_string())?;
    if source.columns != cols {
        return Err(format!("table {name} schema differs from its stored data"));
    }
    Ok((path, parts))
}

pub(super) fn apply_ddl(database: &Path, ddl: &Ddl) -> Result<(), String> {
    match ddl {
        Ddl::ShowTables | Ddl::ShowCreateTable { .. } | Ddl::DescribeTable { .. } => {
            catalog::apply(database, ddl)
        }
        Ddl::CreateTable { name, columns: definitions, if_not_exists } => {
            let _lock = lock(database)?;
            let schema = database.join(format!("{name}.schema"));
            if schema.exists() {
                if !if_not_exists {
                    return Err(format!("table already exists: {name}"));
                }
                let existing = columns(database, name)?;
                validated_parts(database, name, &existing)?;
                return Ok(());
            }
            let parts = data_path(database, name);
            fs::create_dir(&parts)
                .map_err(|error| format!("cannot create table data {}: {error}", parts.display()))?;
            let cols: Vec<Column> = definitions.iter().map(|column| Column {
                name: column.name.clone(),
                data_type: column.data_type.clone(),
            }).collect();
            let result = write_wide_part(&parts.join("all_0_0_0"), &cols, &[])
                .map_err(|error| error.to_string())
                .and_then(|_| catalog::apply(database, ddl));
            if result.is_err() {
                let _ = fs::remove_dir_all(&parts);
            }
            result
        }
        _ => Err("database mode currently supports CREATE, SHOW and DESCRIBE; ALTER, DROP and RENAME are not supported".into()),
    }
}

pub(super) fn insert(database: &Path, insert: &Insert) -> Result<(), String> {
    let _lock = lock(database)?;
    let cols = columns(database, &insert.table)?;
    let (parts_dir, parts) = validated_parts(database, &insert.table, &cols)?;
    let indices: Vec<usize> = match &insert.columns {
        Some(names) => names.iter().map(|name| {
            cols.iter().position(|column| column.name == *name)
                .ok_or_else(|| format!("unknown INSERT column: {name}"))
        }).collect::<Result<Vec<_>, _>>()?,
        None => (0..cols.len()).collect(),
    };
    let mut rows = Vec::with_capacity(insert.rows.len());
    for source in &insert.rows {
        if source.len() != indices.len() {
            return Err("INSERT row width does not match table schema".into());
        }
        let mut values = vec![None; cols.len()];
        for (value, &index) in source.iter().zip(&indices) {
            values[index] = value.clone();
        }
        for (column, value) in cols.iter().zip(&values) {
            if value.is_none() && !column.data_type.starts_with("Nullable(") {
                return Err(format!("missing or NULL value for {}", column.name));
            }
        }
        rows.push(values);
    }
    if parts.len() >= 10_000 {
        return Err("database table has reached its 10000-part limit".into());
    }
    let mut last = 0u64;
    for part in parts {
        let name = part.file_name().and_then(|name| name.to_str())
            .ok_or("invalid stored part filename")?;
        let span = name.strip_prefix("all_").and_then(|name| name.strip_suffix("_0"))
            .and_then(|name| name.split_once('_'))
            .ok_or_else(|| format!("unexpected stored part name: {name}"))?;
        let first = span.0.parse::<u64>().map_err(|_| "invalid local part block number")?;
        let second = span.1.parse::<u64>().map_err(|_| "invalid local part block number")?;
        if first != second {
            return Err("non-local part block ranges cannot be appended".into());
        }
        last = last.max(first);
    }
    let next = last.checked_add(1).ok_or("part block number overflow")?;
    let destination = parts_dir.join(format!("all_{next}_{next}_0"));
    if destination.exists() {
        return Err("destination part already exists".into());
    }
    let staging = parts_dir.join(format!(
        ".pending-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        write_wide_part(&staging, &cols, &rows).map_err(|error| error.to_string())?;
        verify_wide_part(&staging).map_err(|error| error.to_string())?;
        fs::rename(&staging, &destination)
            .map_err(|error| format!("cannot publish inserted rows: {error}"))?;
        fs::File::open(&parts_dir).and_then(|dir| dir.sync_all())
            .map_err(|error| format!("cannot sync table data: {error}"))?;
        Ok::<(), String>(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    println!("Inserted {} rows into {}", rows.len(), insert.table);
    Ok(())
}
