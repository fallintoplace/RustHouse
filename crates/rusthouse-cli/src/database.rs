//! Single-writer local database mode.
//! Tables use existing .schema metadata and immutable, atomically published wide parts.
use crate::catalog;
use rusthouse_query::{Ddl, Delete, Insert, read_matching_rows};
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
        .map_err(|error| {
            format!(
                "cannot acquire database write lock {}: {error}",
                path.display()
            )
        })?;
    Ok(WriteLock { path, _file: file })
}

fn data_path(database: &Path, name: &str) -> PathBuf {
    database.join(format!("{name}.parts"))
}

fn columns(database: &Path, name: &str) -> Result<Vec<Column>, String> {
    let schema = catalog::read_schema(database, name)?;
    Ok(schema
        .into_iter()
        .map(|col| Column {
            name: col.name,
            data_type: col.data_type,
        })
        .collect())
}

fn validated_parts(
    database: &Path,
    name: &str,
    cols: &[Column],
) -> Result<(PathBuf, Vec<PathBuf>), String> {
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

/// Resolve a named SQL table to its immutable on-disk parts.
pub(super) fn select_source(database: &Path, table: &str) -> Result<PathBuf, String> {
    let cols = columns(database, table)?;
    let (path, _) = validated_parts(database, table, &cols)?;
    Ok(path)
}

fn existing_path(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn drop_table(database: &Path, name: &str, if_exists: bool) -> Result<(), String> {
    let _lock = lock(database)?;
    let schema = database.join(format!("{name}.schema"));
    let parts = data_path(database, name);
    if !existing_path(&schema)? {
        if existing_path(&parts)? {
            return Err(format!("table {name} has orphaned data; refusing DROP"));
        }
        if if_exists {
            return Ok(());
        }
        return Err(format!("unknown database table: {name}"));
    }
    let cols = columns(database, name)?;
    validated_parts(database, name, &cols)?;

    // Hide the schema first. Interrupted DROP cannot attach old data to a new table.
    let tombstone = database.join(format!(
        ".{name}.drop-{}-{}.schema",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
    ));
    if existing_path(&tombstone)? {
        return Err(format!("stale DROP metadata at {}", tombstone.display()));
    }
    fs::rename(&schema, &tombstone)
        .map_err(|error| format!("cannot hide table schema {name}: {error}"))?;
    if let Err(error) = fs::remove_dir_all(&parts) {
        return Err(format!(
            "cannot finish DROP for {name}: {error}; recovery schema is at {}",
            tombstone.display()
        ));
    }
    fs::remove_file(&tombstone)
        .map_err(|error| format!("cannot remove dropped schema for {name}: {error}"))?;
    fs::File::open(database)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| format!("table {name} was dropped but sync failed: {error}"))?;
    println!("Dropped table {name}");
    Ok(())
}

fn rename_table(database: &Path, from: &str, to: &str) -> Result<(), String> {
    if from == to {
        return Err("source and destination table names must differ".into());
    }
    let _lock = lock(database)?;
    let cols = columns(database, from)?;
    let (source_parts, _) = validated_parts(database, from, &cols)?;
    let source_schema = database.join(format!("{from}.schema"));
    let destination_schema = database.join(format!("{to}.schema"));
    let destination_parts = data_path(database, to);
    if existing_path(&destination_schema)? || existing_path(&destination_parts)? {
        return Err(format!("table already exists or has orphaned data: {to}"));
    }

    // Two paths cannot be renamed atomically together. Roll back a failed
    // schema rename and never overwrite an existing destination.
    fs::rename(&source_parts, &destination_parts)
        .map_err(|error| format!("cannot move table data from {from}: {error}"))?;
    if let Err(error) = fs::rename(&source_schema, &destination_schema) {
        fs::rename(&destination_parts, &source_parts).map_err(|rollback| {
            format!("cannot rename schema from {from}: {error}; data rollback failed: {rollback}")
        })?;
        return Err(format!("cannot rename schema from {from}: {error}"));
    }
    fs::File::open(database)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| format!("table {from} was renamed but sync failed: {error}"))?;
    println!("Renamed table {from} to {to}");
    Ok(())
}

/// Replace all stored rows with one empty part while preserving the schema.
/// This is a single-writer operation, not an online snapshot transaction.
fn truncate_table(database: &Path, name: &str) -> Result<(), String> {
    let _lock = lock(database)?;
    let cols = columns(database, name)?;
    let (parts, _) = validated_parts(database, name, &cols)?;
    let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
    let staging = database.join(format!(
        ".{name}.truncate-new-{}-{sequence}",
        std::process::id()
    ));
    let retired = database.join(format!(
        ".{name}.truncate-old-{}-{sequence}",
        std::process::id()
    ));
    if existing_path(&staging)? || existing_path(&retired)? {
        return Err("TRUNCATE staging path already exists".into());
    }
    fs::create_dir(&staging)
        .map_err(|error| format!("cannot stage TRUNCATE for {name}: {error}"))?;
    let prepared = write_wide_part(&staging.join("all_0_0_0"), &cols, &[])
        .map_err(|error| error.to_string())
        .and_then(|_| {
            verify_wide_part(staging.join("all_0_0_0")).map_err(|error| error.to_string())
        });
    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = fs::rename(&parts, &retired) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("cannot retire stored rows of {name}: {error}"));
    }
    if let Err(error) = fs::rename(&staging, &parts) {
        let rollback = fs::rename(&retired, &parts);
        let _ = fs::remove_dir_all(&staging);
        return match rollback {
            Ok(()) => Err(format!("cannot publish empty table {name}: {error}")),
            Err(rollback) => Err(format!(
                "cannot publish empty table {name}: {error}; rollback failed: {rollback}"
            )),
        };
    }
    fs::File::open(database)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| format!("table {name} was truncated but sync failed: {error}"))?;
    fs::remove_dir_all(&retired)
        .map_err(|error| format!("table {name} was truncated but cleanup failed: {error}"))?;
    println!("Truncated table {name}");
    Ok(())
}

pub(super) fn apply_ddl(database: &Path, ddl: &Ddl) -> Result<(), String> {
    match ddl {
        Ddl::ShowTables | Ddl::ShowCreateTable { .. } | Ddl::DescribeTable { .. } => {
            catalog::apply(database, ddl)
        }
        Ddl::CreateTable {
            name,
            columns: definitions,
            if_not_exists,
        } => {
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
            fs::create_dir(&parts).map_err(|error| {
                format!("cannot create table data {}: {error}", parts.display())
            })?;
            let cols: Vec<Column> = definitions
                .iter()
                .map(|column| Column {
                    name: column.name.clone(),
                    data_type: column.data_type.clone(),
                })
                .collect();
            let result = write_wide_part(&parts.join("all_0_0_0"), &cols, &[])
                .map_err(|error| error.to_string())
                .and_then(|_| catalog::apply(database, ddl));
            if result.is_err() {
                let _ = fs::remove_dir_all(&parts);
            }
            result
        }
        Ddl::DropTable { name, if_exists } => drop_table(database, name, *if_exists),
        Ddl::TruncateTable { name } => truncate_table(database, name),
        Ddl::RenameTable { from, to } => rename_table(database, from, to),
        _ => Err("database mode does not support ALTER yet".into()),
    }
}

/// Replace one small table snapshot, preserving old data on preparation failures.
fn replace_rows(
    database: &Path,
    name: &str,
    columns: &[Column],
    current: &Path,
    rows: &[Vec<Option<String>>],
) -> Result<(), String> {
    let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
    let staging = database.join(format!(
        ".{name}.rewrite-new-{}-{sequence}",
        std::process::id()
    ));
    let retired = database.join(format!(
        ".{name}.rewrite-old-{}-{sequence}",
        std::process::id()
    ));
    if existing_path(&staging)? || existing_path(&retired)? {
        return Err("SQL mutation staging path already exists".into());
    }
    fs::create_dir(&staging)
        .map_err(|error| format!("cannot stage table rewrite: {error}"))?;
    let prepared = write_wide_part(&staging.join("all_0_0_0"), columns, rows)
        .map_err(|error| error.to_string())
        .and_then(|_| {
            verify_wide_part(staging.join("all_0_0_0")).map_err(|error| error.to_string())
        });
    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = fs::rename(current, &retired) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("cannot retire old table parts: {error}"));
    }
    if let Err(error) = fs::rename(&staging, current) {
        let rollback = fs::rename(&retired, current);
        let _ = fs::remove_dir_all(&staging);
        return match rollback {
            Ok(()) => Err(format!("cannot publish rewritten table: {error}")),
            Err(rollback) => Err(format!(
                "cannot publish rewritten table: {error}; rollback failed: {rollback}"
            )),
        };
    }
    fs::File::open(database)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| format!("table rewrite published but sync failed: {error}"))?;
    fs::remove_dir_all(&retired)
        .map_err(|error| format!("table rewrite published but cleanup failed: {error}"))?;
    Ok(())
}

pub(super) fn delete(database: &Path, delete: &Delete) -> Result<(), String> {
    let _lock = lock(database)?;
    let cols = columns(database, &delete.table)?;
    let (parts_dir, _) = validated_parts(database, &delete.table, &cols)?;
    let existing = read_matching_rows(
        &delete.predicates,
        &delete.or_predicates,
        &parts_dir,
    )
    .map_err(|error| error.to_string())?;
    let count = existing.iter().filter(|(_, matched)| *matched).count();
    if count > 0 {
        let retained = existing
            .into_iter()
            .filter_map(|(row, matched)| (!matched).then_some(row))
            .collect::<Vec<_>>();
        replace_rows(database, &delete.table, &cols, &parts_dir, &retained)?;
    }
    println!("Deleted {count} rows from {}", delete.table);
    Ok(())
}

pub(super) fn insert(database: &Path, insert: &Insert) -> Result<(), String> {
    let _lock = lock(database)?;
    let cols = columns(database, &insert.table)?;
    let (parts_dir, parts) = validated_parts(database, &insert.table, &cols)?;
    let indices: Vec<usize> = match &insert.columns {
        Some(names) => names
            .iter()
            .map(|name| {
                cols.iter()
                    .position(|column| column.name == *name)
                    .ok_or_else(|| format!("unknown INSERT column: {name}"))
            })
            .collect::<Result<Vec<_>, _>>()?,
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
        let name = part
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("invalid stored part filename")?;
        let span = name
            .strip_prefix("all_")
            .and_then(|name| name.strip_suffix("_0"))
            .and_then(|name| name.split_once('_'))
            .ok_or_else(|| format!("unexpected stored part name: {name}"))?;
        let first = span
            .0
            .parse::<u64>()
            .map_err(|_| "invalid local part block number")?;
        let second = span
            .1
            .parse::<u64>()
            .map_err(|_| "invalid local part block number")?;
        if first != second {
            return Err("non-local part block ranges cannot be appended".into());
        }
        last = last.max(first);
    }
    let next = last.checked_add(1).ok_or("part block number overflow")?;
    let destination = parts_dir.join(format!("all_{next}_{next}_0"));
    if existing_path(&destination)? {
        return Err("destination part already exists".into());
    }
    // An interrupted write must not become visible as an active part.
    let staging = database.join(format!(
        ".{}.pending-{}-{}",
        insert.table,
        std::process::id(),
        NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        write_wide_part(&staging, &cols, &rows).map_err(|error| error.to_string())?;
        verify_wide_part(&staging).map_err(|error| error.to_string())?;
        fs::rename(&staging, &destination)
            .map_err(|error| format!("cannot publish inserted rows: {error}"))?;
        fs::File::open(&parts_dir)
            .and_then(|dir| dir.sync_all())
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
