//! Local catalog table data, stored as immutable wide parts.
//! Each INSERT publishes a complete part via a rename after all streams are synced.
use rusthouse_query::Insert;
use rusthouse_storage::{Column, inspect_query_source, query_parts, write_wide_part};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static LAST_PART_ID: AtomicU64 = AtomicU64::new(0);

pub(super) fn path(catalog: &Path, table: &str) -> PathBuf {
    catalog.join(format!("{table}.parts"))
}

pub(super) fn existing_dir(catalog: &Path, table: &str) -> Result<Option<PathBuf>, String> {
    let dir = path(catalog, table);
    match fs::symlink_metadata(&dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot inspect table data for {table}: {error}")),
        Ok(metadata) if metadata.file_type().is_dir() => Ok(Some(dir)),
        Ok(_) => Err(format!("table data for {table} is not a regular directory")),
    }
}

pub(super) fn require_no_data_dir(catalog: &Path, table: &str) -> Result<(), String> {
    if existing_dir(catalog, table)?.is_some() {
        return Err(format!("orphan table data already exists for {table}"));
    }
    Ok(())
}

pub(super) fn ensure_unpopulated(catalog: &Path, table: &str) -> Result<(), String> {
    if let Some(dir) = existing_dir(catalog, table)? {
        if fs::read_dir(&dir)
            .map_err(|error| format!("cannot list table data for {table}: {error}"))?
            .next()
            .transpose()
            .map_err(|error| format!("cannot inspect table data for {table}: {error}"))?
            .is_some()
        {
            return Err(format!(
                "ALTER TABLE on populated table {table} is not supported"
            ));
        }
    }
    Ok(())
}

fn next_id() -> Result<u64, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("invalid system clock: {error}"))?
        .as_nanos();
    let now = u64::try_from(nanos).map_err(|_| "system clock exceeds UInt64 nanoseconds")?;
    let mut previous = LAST_PART_ID.load(Ordering::Relaxed);
    loop {
        let candidate = now.max(
            previous
                .checked_add(1)
                .ok_or("exhausted table part identifiers")?,
        );
        match LAST_PART_ID.compare_exchange_weak(
            previous,
            candidate,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ok(candidate),
            Err(observed) => previous = observed,
        }
    }
}

fn ordered_rows(
    insert: &Insert,
    columns: &[Column],
) -> Result<Vec<Vec<Option<String>>>, String> {
    let positions = if let Some(names) = &insert.columns {
        let positions = names
            .iter()
            .map(|name| {
                columns
                    .iter()
                    .position(|column| &column.name == name)
                    .ok_or_else(|| format!("unknown INSERT column: {name}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (index, column) in columns.iter().enumerate() {
            if !positions.contains(&index) && !column.data_type.starts_with("Nullable(") {
                return Err(format!("required INSERT column omitted: {}", column.name));
            }
        }
        positions
    } else {
        (0..columns.len()).collect()
    };
    insert
        .rows
        .iter()
        .map(|row| {
            if row.len() != positions.len() {
                return Err("INSERT row width does not match table schema".to_owned());
            }
            let mut ordered = vec![None; columns.len()];
            for (value, &position) in row.iter().zip(&positions) {
                ordered[position] = value.clone();
            }
            Ok(ordered)
        })
        .collect()
}

pub(super) fn insert(catalog: &Path, statement: &Insert) -> Result<(), String> {
    let schema = crate::catalog::read_schema(catalog, &statement.table)?;
    let columns = schema
        .into_iter()
        .map(|column| Column {
            name: column.name,
            data_type: column.data_type,
        })
        .collect::<Vec<_>>();
    let rows = ordered_rows(statement, &columns)?;
    let existing = existing_dir(catalog, &statement.table)?;
    if let Some(ref dir) = existing {
        let mut entries = fs::read_dir(dir)
            .map_err(|error| format!("cannot list table parts: {error}"))?;
        if entries.next().transpose().map_err(|error| error.to_string())?.is_some() {
            let metadata = inspect_query_source(dir).map_err(|error| error.to_string())?;
            if metadata.columns != columns {
                return Err("table parts do not match the catalog schema".into());
            }
            if query_parts(dir).map_err(|error| error.to_string())?.len() >= 10_000 {
                return Err("table supports at most 10000 parts".into());
            }
        }
    }
    let dir = path(catalog, &statement.table);
    if existing.is_none() {
        fs::create_dir(&dir)
            .map_err(|error| format!("cannot create table parts: {error}"))?;
    }
    // Pending parts live outside the scanned table directory. Readers only see
    // fully published parts, even if INSERT and SELECT overlap.
    let id = next_id()?;
    let pending = catalog.join(format!(
        ".{}.insert.{}.{}",
        statement.table,
        std::process::id(),
        id
    ));
    if fs::symlink_metadata(&pending).is_ok() {
        return Err("INSERT temporary path already exists".into());
    }
    let published = dir.join(format!("all_{id}_{id}_0"));
    if fs::symlink_metadata(&published).is_ok() {
        return Err("INSERT part identifier already exists".into());
    }
    let result = (|| -> Result<(), String> {
        write_wide_part(&pending, &columns, &rows).map_err(|error| error.to_string())?;
        fs::rename(&pending, &published)
            .map_err(|error| format!("cannot publish inserted rows: {error}"))?;
        fs::File::open(&dir)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("cannot sync table directory: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&pending);
    }
    result?;
    println!("Inserted {} rows into {}", rows.len(), statement.table);
    Ok(())
}
