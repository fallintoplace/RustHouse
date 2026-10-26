//! Bounded copy-on-write snapshots for local SQL tables.
//! The active pointer is atomically replaced only after all new parts are verified.
use super::{NEXT_STAGE, columns, crash_if_requested, data_path, lock, validated_parts};
use rusthouse_storage::{
    Column, NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source, verify_wide_part,
    write_wide_part,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

const MAX_REWRITE_ROWS: u64 = 25_000;
const MAX_REWRITE_BYTES: usize = 16 * 1024 * 1024;
const PART_ROWS: usize = 4096;

type StoredRows = Vec<Vec<Option<String>>>;

fn numeric_value(values: &NumericColumn, index: usize) -> String {
    match values {
        NumericColumn::UInt8(v) => v[index].to_string(),
        NumericColumn::UInt16(v) => v[index].to_string(),
        NumericColumn::UInt32(v) => v[index].to_string(),
        NumericColumn::UInt64(v) => v[index].to_string(),
        NumericColumn::UInt128(v) => v[index].to_string(),
        NumericColumn::Int8(v) => v[index].to_string(),
        NumericColumn::Int16(v) => v[index].to_string(),
        NumericColumn::Int32(v) => v[index].to_string(),
        NumericColumn::Int64(v) => v[index].to_string(),
        NumericColumn::Int128(v) => v[index].to_string(),
        NumericColumn::Float32(v) => v[index].to_string(),
        NumericColumn::Float64(v) => v[index].to_string(),
    }
}

fn text_value(bytes: Option<&[u8]>) -> Result<Option<String>, String> {
    let bytes = bytes.ok_or("invalid String row offset")?;
    let decoded =
        std::str::from_utf8(bytes).map_err(|_| "database mutation requires UTF-8 String values")?;
    Ok(Some(decoded.to_owned()))
}

fn column_value(column: &TypedColumn, row: usize) -> Result<Option<String>, String> {
    match column {
        TypedColumn::Numeric(values) => Ok(Some(numeric_value(values, row))),
        TypedColumn::Text(values) => text_value(values.get(row)),
        TypedColumn::NullableNumeric { values, nulls } => {
            if nulls[row] != 0 {
                Ok(None)
            } else {
                Ok(Some(numeric_value(values, row)))
            }
        }
        TypedColumn::NullableText { values, nulls } => {
            if nulls[row] != 0 {
                Ok(None)
            } else {
                text_value(values.get(row))
            }
        }
        TypedColumn::Uuid(_) => Err("writable tables do not support UUID columns".into()),
    }
}

fn read_rows(
    source: &Path,
    cols: &[Column],
    mut transform: impl FnMut(&[TypedColumn], usize, &mut Vec<Option<String>>) -> Result<bool, String>,
) -> Result<(StoredRows, usize), String> {
    let metadata = inspect_query_source(source).map_err(|error| error.to_string())?;
    if metadata.rows > MAX_REWRITE_ROWS {
        return Err(format!(
            "table rewrite supports at most {MAX_REWRITE_ROWS} rows"
        ));
    }
    let names = cols
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(source, &names).map_err(|error| error.to_string())?;
    let mut rows = Vec::new();
    let mut scanned = 0usize;
    let mut bytes = 0usize;
    while let Some(batch) = reader.next_batch().map_err(|error| error.to_string())? {
        let len = batch.first().map_or(0, TypedColumn::len);
        if batch.len() != cols.len() || batch.iter().any(|column| column.len() != len) {
            return Err("corrupt database scan batch".into());
        }
        for index in 0..len {
            scanned += 1;
            if scanned as u64 > MAX_REWRITE_ROWS {
                return Err("database rewrite exceeds row limit".into());
            }
            let mut values = batch
                .iter()
                .map(|column| column_value(column, index))
                .collect::<Result<Vec<_>, _>>()?;
            if transform(&batch, index, &mut values)? {
                if values.len() != cols.len() {
                    return Err("row rewrite changed table width".into());
                }
                for value in values.iter().flatten() {
                    bytes = bytes
                        .checked_add(value.len())
                        .ok_or("database rewrite exceeds memory limit")?;
                    if bytes > MAX_REWRITE_BYTES {
                        return Err("database rewrite exceeds 16 MiB of values".into());
                    }
                }
                rows.push(values);
            }
        }
    }
    if scanned as u64 != metadata.rows {
        return Err("database rewrite read an inconsistent row count".into());
    }
    Ok((rows, scanned))
}

fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| format!("cannot sync {}: {error}", path.display()))
}

fn generations_dir(database: &Path, table: &str) -> Result<std::path::PathBuf, String> {
    let path = data_path(database, table).join(".rusthouse-versions");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            Err("database snapshot directory must not be a symlink or file".into())
        }
        Ok(_) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&path)
                .map_err(|error| format!("cannot create snapshot directory: {error}"))?;
            sync_directory(&data_path(database, table))?;
            Ok(path)
        }
        Err(error) => Err(format!("cannot inspect snapshot directory: {error}")),
    }
}

/// Publish a verified immutable generation and atomically swap its pointer.
/// The staging writer may stream parts without holding the whole table in memory.
/// Caller must hold the database write lock and validate the existing table.
fn publish_snapshot_with(
    database: &Path,
    table: &str,
    prepare: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let versions = generations_dir(database, table)?;
    let mut highest = 0u64;
    for entry in fs::read_dir(&versions).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let label = entry.file_name();
        let Some(label) = label.to_str() else {
            continue;
        };
        if let Some(id) = label
            .strip_prefix('g')
            .and_then(|tail| tail.parse::<u64>().ok())
        {
            highest = highest.max(id);
        }
    }
    let next = highest
        .checked_add(1)
        .ok_or("database snapshot generation overflow")?;
    let name = format!("g{next}");
    let destination = versions.join(&name);
    let staging = versions.join(format!(
        ".pending-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir(&staging)
        .map_err(|error| format!("cannot stage new database snapshot: {error}"))?;
    let prepared = (|| -> Result<(), String> {
        prepare(&staging)?;
        sync_directory(&staging)?;
        crash_if_requested("snapshot-after-staging");
        Ok(())
    })();
    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if destination.exists() {
        let _ = fs::remove_dir_all(&staging);
        return Err("snapshot generation already exists".into());
    }
    fs::rename(&staging, &destination)
        .map_err(|error| format!("cannot publish database generation: {error}"))?;
    sync_directory(&versions)?;
    crash_if_requested("snapshot-after-generation");

    let root = data_path(database, table);
    let pointer = root.join(".rusthouse-active");
    let pending = root.join(format!(
        ".rusthouse-active.tmp-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&pending)
        .map_err(|error| format!("cannot create snapshot pointer: {error}"))?;
    file.write_all(format!("{name}\n").as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("cannot persist snapshot pointer: {error}"))?;
    drop(file);
    crash_if_requested("snapshot-before-pointer");
    fs::rename(&pending, &pointer)
        .map_err(|error| format!("cannot activate database snapshot: {error}"))?;
    crash_if_requested("snapshot-after-pointer");
    sync_directory(&root)?;
    Ok(())
}

/// Publish a bounded in-memory rewrite for UPDATE, DELETE, and TRUNCATE.
/// The staging and pointer protocol is shared with streaming compaction.
pub(super) fn publish_snapshot(
    database: &Path,
    table: &str,
    cols: &[Column],
    rows: &[Vec<Option<String>>],
) -> Result<(), String> {
    publish_snapshot_with(database, table, |staging| {
        if rows.is_empty() {
            let part = staging.join("all_0_0_0");
            write_wide_part(&part, cols, &[]).map_err(|error| error.to_string())?;
            verify_wide_part(&part).map_err(|error| error.to_string())?;
        } else {
            for (index, chunk) in rows.chunks(PART_ROWS).enumerate() {
                let number = index + 1;
                let part = staging.join(format!("all_{number}_{number}_0"));
                write_wide_part(&part, cols, chunk).map_err(|error| error.to_string())?;
                verify_wide_part(&part).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    })
}

/// Append multiple new parts as one snapshot generation. Reuse immutable
/// existing part files through checked hard links, never partial live renames.
pub(super) fn append_parts(
    database: &Path,
    table: &str,
    cols: &[Column],
    existing: &[std::path::PathBuf],
    rows: &[Vec<Option<String>>],
) -> Result<(), String> {
    let count = rows.len().div_ceil(PART_ROWS);
    if existing
        .len()
        .checked_add(count)
        .is_none_or(|parts| parts > 10_000)
    {
        return Err("database table would exceed its 10000-part limit".into());
    }
    let mut last = 0u64;
    for part in existing {
        let name = part
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("invalid stored part filename")?;
        let (first, second) = name
            .strip_prefix("all_")
            .and_then(|name| name.strip_suffix("_0"))
            .and_then(|name| name.split_once('_'))
            .ok_or_else(|| format!("unexpected stored part name: {name}"))?;
        let first = first.parse::<u64>().map_err(|_| "invalid part number")?;
        let second = second.parse::<u64>().map_err(|_| "invalid part number")?;
        if first != second {
            return Err("non-local part block ranges cannot be appended".into());
        }
        last = last.max(first);
    }

    publish_snapshot_with(database, table, |staging| {
        for source in existing {
            verify_wide_part(source).map_err(|error| error.to_string())?;
            let name = source.file_name().ok_or("invalid stored part filename")?;
            let destination = staging.join(name);
            fs::create_dir(&destination)
                .map_err(|error| format!("cannot stage existing part: {error}"))?;
            for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                let kind = entry.file_type().map_err(|error| error.to_string())?;
                if !kind.is_file() {
                    return Err("cannot hard-link a non-regular part entry".into());
                }
                fs::hard_link(entry.path(), destination.join(entry.file_name()))
                    .map_err(|error| format!("cannot stage immutable part file: {error}"))?;
            }
            sync_directory(&destination)?;
        }
        for (index, chunk) in rows.chunks(PART_ROWS).enumerate() {
            let next = last
                .checked_add(u64::try_from(index).map_err(|_| "part index overflow")?)
                .and_then(|number| number.checked_add(1))
                .ok_or("part block number overflow")?;
            let part = staging.join(format!("all_{next}_{next}_0"));
            write_wide_part(&part, cols, chunk).map_err(|error| error.to_string())?;
            verify_wide_part(&part).map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

/// Rewrite a local table while preserving the previous active snapshot on failure.
pub(super) fn rewrite_rows(
    database: &Path,
    table: &str,
    transform: impl FnMut(&[TypedColumn], usize, &mut Vec<Option<String>>) -> Result<bool, String>,
) -> Result<(usize, usize), String> {
    let _lock = lock(database)?;
    let cols = columns(database, table)?;
    let (source, _) = validated_parts(database, table, &cols)?;
    let (rows, read) = read_rows(&source, &cols, transform)?;
    let retained = rows.len();
    publish_snapshot(database, table, &cols, &rows)?;
    Ok((read, retained))
}

/// Stream immutable table rows into verified compacted parts. Keep only
/// PART_ROWS decoded rows in memory, independent of the full table size.
pub(super) fn compact_table(database: &Path, table: &str) -> Result<(), String> {
    let _lock = lock(database)?;
    let cols = columns(database, table)?;
    let (source, parts) = validated_parts(database, table, &cols)?;
    if parts.len() <= 1 {
        println!("Table {table} is already compact");
        return Ok(());
    }
    let expected = inspect_query_source(&source)
        .map_err(|error| error.to_string())?
        .rows;
    let names = cols
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(&source, &names).map_err(|error| error.to_string())?;
    let mut scanned = 0u64;
    publish_snapshot_with(database, table, |staging| {
        let mut buffer = Vec::with_capacity(PART_ROWS);
        let mut part_number = 0u64;
        let mut flush = |rows: &mut Vec<Vec<Option<String>>>| -> Result<(), String> {
            if rows.is_empty() {
                return Ok(());
            }
            part_number = part_number.checked_add(1).ok_or("part number overflow")?;
            let part = staging.join(format!("all_{part_number}_{part_number}_0"));
            write_wide_part(&part, &cols, rows).map_err(|error| error.to_string())?;
            verify_wide_part(&part).map_err(|error| error.to_string())?;
            rows.clear();
            Ok(())
        };
        while let Some(batch) = reader.next_batch().map_err(|error| error.to_string())? {
            let length = batch.first().map_or(0, TypedColumn::len);
            if batch.len() != cols.len() || batch.iter().any(|column| column.len() != length) {
                return Err("corrupt database scan batch".into());
            }
            for index in 0..length {
                let row = batch
                    .iter()
                    .map(|column| column_value(column, index))
                    .collect::<Result<Vec<_>, _>>()?;
                buffer.push(row);
                scanned = scanned.checked_add(1).ok_or("table row count overflow")?;
                if buffer.len() == PART_ROWS {
                    flush(&mut buffer)?;
                }
            }
        }
        if scanned != expected {
            return Err("database compaction read an inconsistent row count".into());
        }
        flush(&mut buffer)?;
        if scanned == 0 {
            let part = staging.join("all_0_0_0");
            write_wide_part(&part, &cols, &[]).map_err(|error| error.to_string())?;
            verify_wide_part(&part).map_err(|error| error.to_string())?;
        }
        Ok(())
    })?;
    println!("Optimized table {table}: read {scanned} rows, retained {scanned}");
    Ok(())
}
