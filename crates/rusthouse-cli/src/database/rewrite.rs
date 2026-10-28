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

const MAX_REWRITE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PART_BUFFER_BYTES: usize = 4 * 1024 * 1024;
const MAX_STREAMED_PART_BYTES: usize = 64 * 1024 * 1024;
const MAX_STREAMED_COLUMN_BYTES: usize = 7 * 1024 * 1024;
const MAX_REWRITE_PARTS: u64 = 16_384;
// The writer accepts 65,536 rows. Rewrites also retain a 4 MiB value budget.
const PART_ROWS: usize = 65_536;

// Debug-only simulated publication errors allow integration tests to exercise
// cleanup and recovery without relying on platform-dependent permissions.
#[cfg(debug_assertions)]
fn fail_publication_at(point: &str) -> Result<(), String> {
    if std::env::var("RUSTHOUSE_DEBUG_FAIL_AT").is_ok_and(|value| value == point) {
        return Err(format!("injected snapshot publication failure: {point}"));
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn fail_publication_at(_point: &str) -> Result<(), String> {
    Ok(())
}

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
    let published = fail_publication_at("snapshot-before-generation-rename").and_then(|_| {
        fs::rename(&staging, &destination)
            .map_err(|error| format!("cannot publish database generation: {error}"))
    });
    if let Err(error) = published {
        // No generation has become active. Keep the old snapshot readable
        // and do not accumulate failed multi-part staging directories.
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    sync_directory(&versions)?;
    crash_if_requested("snapshot-after-generation");

    let root = data_path(database, table);
    let pointer = root.join(".rusthouse-active");
    let pending = root.join(format!(
        ".rusthouse-active.tmp-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let prepared_pointer = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)
            .map_err(|error| format!("cannot create snapshot pointer: {error}"))?;
        file.write_all(format!("{name}\n").as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("cannot persist snapshot pointer: {error}"))
    })();
    if let Err(error) = prepared_pointer {
        let _ = fs::remove_file(&pending);
        return Err(error);
    }
    crash_if_requested("snapshot-before-pointer");
    let activated = fail_publication_at("snapshot-before-pointer-rename").and_then(|_| {
        fs::rename(&pending, &pointer)
            .map_err(|error| format!("cannot activate database snapshot: {error}"))
    });
    if let Err(error) = activated {
        // The pointer still names the previously committed generation.
        // A newer orphaned generation is retained for offline recovery.
        let _ = fs::remove_file(&pending);
        return Err(error);
    }
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

/// Existing immutable parts remain verified before being hard-linked into a
/// new private generation. Work on independent parts can run concurrently.
fn stage_existing_part(source: &Path, staging: &Path) -> Result<(), String> {
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
    Ok(())
}

fn stage_existing_parts_with_workers(
    existing: &[std::path::PathBuf],
    staging: &Path,
    workers: usize,
) -> Result<(), String> {
    if workers <= 1 || existing.len() < 16 {
        for source in existing {
            stage_existing_part(source, staging)?;
        }
        return Ok(());
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let completed = std::thread::scope(|scope| {
        let handles = (0..workers).map(|_| {
            let next = &next;
            scope.spawn(move || -> Result<(), String> {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= existing.len() {
                        return Ok(());
                    }
                    stage_existing_part(&existing[index], staging)?;
                }
            })
        }).collect::<Vec<_>>();
        handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| {
            Err("immutable part staging worker panicked".into())
        })).collect::<Vec<_>>()
    });
    for result in completed {
        result?;
    }
    Ok(())
}

fn stage_existing_parts(
    existing: &[std::path::PathBuf],
    staging: &Path,
) -> Result<(), String> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |available| available.get()).min(4);
    stage_existing_parts_with_workers(existing, staging, workers)
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
        .is_none_or(|parts| parts > 16_384)
    {
        return Err("database table would exceed its 16384-part limit".into());
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
        stage_existing_parts(existing, staging)?;
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

/// Append a stream of schema-ordered rows as one verified snapshot.
/// An input or storage error leaves the previous active snapshot unchanged.
pub(super) fn append_parts_stream(
    database: &Path,
    table: &str,
    cols: &[Column],
    existing: &[std::path::PathBuf],
    mut next_row: impl FnMut() -> Result<Option<Vec<Option<String>>>, String>,
) -> Result<usize, String> {
    let mut last = 0u64;
    for source in existing {
        let name = source
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

    let mut count = 0usize;
    publish_snapshot_with(database, table, |staging| {
        stage_existing_parts(existing, staging)?;
        let mut buffer: StoredRows = Vec::with_capacity(PART_ROWS);
        let mut buffered_bytes = 0usize;
        let mut column_bytes = vec![0usize; cols.len()];
        let mut part_index = 0usize;
        let mut flush = |rows: &mut StoredRows,
                         bytes: &mut usize,
                         per_column: &mut [usize]|
         -> Result<(), String> {
            if rows.is_empty() {
                return Ok(());
            }
            part_index = part_index.checked_add(1).ok_or("part index overflow")?;
            if existing
                .len()
                .checked_add(part_index)
                .is_none_or(|total| total > 16_384)
            {
                return Err("database table would exceed its 16384-part limit".into());
            }
            let number = last
                .checked_add(u64::try_from(part_index).map_err(|_| "part index overflow")?)
                .ok_or("part block number overflow")?;
            let part = staging.join(format!("all_{number}_{number}_0"));
            write_wide_part(&part, cols, rows).map_err(|error| error.to_string())?;
            verify_wide_part(&part).map_err(|error| error.to_string())?;
            rows.clear();
            *bytes = 0;
            per_column.fill(0);
            Ok(())
        };
        while let Some(row) = next_row()? {
            if row.len() != cols.len() {
                return Err("COPY row width does not match table schema".into());
            }
            let estimate = |value: &Option<String>| {
                value
                    .as_ref()
                    .map_or(16, |value| value.len().saturating_add(16))
            };
            let row_bytes = row.iter().try_fold(0usize, |size, value| {
                size.checked_add(estimate(value))
                    .ok_or_else(|| "COPY record size overflow".to_string())
            })?;
            if row_bytes > MAX_STREAMED_PART_BYTES
                || row
                    .iter()
                    .any(|value| estimate(value) > MAX_STREAMED_COLUMN_BYTES)
            {
                return Err("COPY row exceeds streaming part budget".into());
            }
            let column_overflow = row.iter().zip(&column_bytes).any(|(value, used)| {
                used.checked_add(estimate(value))
                    .is_none_or(|size| size > MAX_STREAMED_COLUMN_BYTES)
            });
            if !buffer.is_empty()
                && (buffer.len() >= PART_ROWS
                    || buffered_bytes
                        .checked_add(row_bytes)
                        .is_none_or(|size| size > MAX_STREAMED_PART_BYTES)
                    || column_overflow)
            {
                flush(&mut buffer, &mut buffered_bytes, &mut column_bytes)?;
            }
            buffered_bytes = buffered_bytes
                .checked_add(row_bytes)
                .ok_or("COPY batch size overflow")?;
            for (value, used) in row.iter().zip(&mut column_bytes) {
                *used = used
                    .checked_add(estimate(value))
                    .ok_or("COPY column size overflow")?;
            }
            buffer.push(row);
            count = count.checked_add(1).ok_or("COPY row count overflow")?;
        }
        flush(&mut buffer, &mut buffered_bytes, &mut column_bytes)?;
        Ok(())
    })?;
    Ok(count)
}

/// Rewrite a local table using bounded batches. Keep the previous active
/// snapshot visible until every replacement part has been written and verified.
pub(super) fn rewrite_rows(
    database: &Path,
    table: &str,
    mut transform: impl FnMut(&[TypedColumn], usize, &mut Vec<Option<String>>) -> Result<bool, String>,
) -> Result<(usize, usize), String> {
    let _lock = lock(database)?;
    let cols = columns(database, table)?;
    let (source, _) = validated_parts(database, table, &cols)?;
    let expected = inspect_query_source(&source)
        .map_err(|error| error.to_string())?
        .rows;
    let names = cols
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(&source, &names).map_err(|error| error.to_string())?;
    let mut scanned = 0usize;
    let mut retained = 0usize;
    publish_snapshot_with(database, table, |staging| {
        let mut buffer: StoredRows = Vec::with_capacity(PART_ROWS);
        let mut buffered_bytes = 0usize;
        let mut part_number = 0u64;
        let mut flush = |rows: &mut StoredRows, bytes: &mut usize| -> Result<(), String> {
            if rows.is_empty() {
                return Ok(());
            }
            part_number = part_number.checked_add(1).ok_or("part number overflow")?;
            if part_number > MAX_REWRITE_PARTS {
                return Err("database table would exceed its 16384-part limit".into());
            }
            let part = staging.join(format!("all_{part_number}_{part_number}_0"));
            write_wide_part(&part, &cols, rows).map_err(|error| error.to_string())?;
            verify_wide_part(&part).map_err(|error| error.to_string())?;
            rows.clear();
            *bytes = 0;
            Ok(())
        };
        while let Some(batch) = reader.next_batch().map_err(|error| error.to_string())? {
            let length = batch.first().map_or(0, TypedColumn::len);
            if batch.len() != cols.len() || batch.iter().any(|col| col.len() != length) {
                return Err("corrupt database scan batch".into());
            }
            for index in 0..length {
                scanned = scanned
                    .checked_add(1)
                    .ok_or("database row count overflow")?;
                let mut values = batch
                    .iter()
                    .map(|col| column_value(col, index))
                    .collect::<Result<Vec<_>, _>>()?;
                if !transform(&batch, index, &mut values)? {
                    continue;
                }
                if values.len() != cols.len() {
                    return Err("row rewrite changed table width".into());
                }
                let row_bytes = values.iter().flatten().try_fold(0usize, |sum, value| {
                    sum.checked_add(value.len())
                        .ok_or_else(|| "database rewrite exceeds memory limit".to_string())
                })?;
                if row_bytes > MAX_REWRITE_BYTES {
                    return Err("database rewrite exceeds 16 MiB in a single row".into());
                }
                if !buffer.is_empty()
                    && (buffer.len() >= PART_ROWS
                        || buffered_bytes
                            .checked_add(row_bytes)
                            .is_none_or(|size| size > MAX_PART_BUFFER_BYTES))
                {
                    flush(&mut buffer, &mut buffered_bytes)?;
                }
                buffered_bytes = buffered_bytes
                    .checked_add(row_bytes)
                    .ok_or("database rewrite exceeds memory limit")?;
                buffer.push(values);
                retained = retained
                    .checked_add(1)
                    .ok_or("database row count overflow")?;
            }
        }
        if u64::try_from(scanned).ok() != Some(expected) {
            return Err("database rewrite read an inconsistent row count".into());
        }
        flush(&mut buffer, &mut buffered_bytes)?;
        if retained == 0 {
            let part = staging.join("all_0_0_0");
            write_wide_part(&part, &cols, &[]).map_err(|error| error.to_string())?;
            verify_wide_part(&part).map_err(|error| error.to_string())?;
        }
        Ok(())
    })?;
    Ok((scanned, retained))
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
        let mut buffered_bytes = 0usize;
        let mut part_number = 0u64;
        let mut flush =
            |rows: &mut Vec<Vec<Option<String>>>, bytes: &mut usize| -> Result<(), String> {
                if rows.is_empty() {
                    return Ok(());
                }
                part_number = part_number.checked_add(1).ok_or("part number overflow")?;
                let part = staging.join(format!("all_{part_number}_{part_number}_0"));
                write_wide_part(&part, &cols, rows).map_err(|error| error.to_string())?;
                verify_wide_part(&part).map_err(|error| error.to_string())?;
                rows.clear();
                *bytes = 0;
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
                let row_bytes = row.iter().flatten().try_fold(0usize, |total, value| {
                    total
                        .checked_add(value.len())
                        .ok_or_else(|| "compaction row size overflow".to_string())
                })?;
                if !buffer.is_empty()
                    && (buffer.len() >= PART_ROWS
                        || buffered_bytes
                            .checked_add(row_bytes)
                            .is_none_or(|bytes| bytes > MAX_PART_BUFFER_BYTES))
                {
                    flush(&mut buffer, &mut buffered_bytes)?;
                }
                buffered_bytes = buffered_bytes
                    .checked_add(row_bytes)
                    .ok_or("compaction batch size overflow")?;
                buffer.push(row);
                scanned = scanned.checked_add(1).ok_or("table row count overflow")?;
            }
        }
        if scanned != expected {
            return Err("database compaction read an inconsistent row count".into());
        }
        flush(&mut buffer, &mut buffered_bytes)?;
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

#[cfg(test)]
mod parallel_snapshot_staging_tests {
    use super::*;
    use crate::database::Column;

    #[test]
    fn wide_snapshot_parts_are_verified_and_hard_linked_with_bounded_workers() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-part-staging-workers-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let schema = [Column { name: "value".into(), data_type: "UInt64".into() }];
        let mut sources = Vec::new();
        for index in 0..24 {
            let path = root.join(format!("all_{index}_{index}_0"));
            write_wide_part(&path, &schema, &[vec![Some(index.to_string())]]).unwrap();
            sources.push(path);
        }
        let staged = root.join("staging");
        fs::create_dir_all(&staged).unwrap();
        stage_existing_parts_with_workers(&sources, &staged, 4).unwrap();
        for source in &sources {
            let copied = staged.join(source.file_name().unwrap());
            verify_wide_part(&copied).unwrap();
            let original = fs::read(source.join("count.txt")).unwrap();
            assert_eq!(fs::read(copied.join("count.txt")).unwrap(), original);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
