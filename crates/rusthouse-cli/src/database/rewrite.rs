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
// Retain the original row chunk bound for ordinary rewrites and INSERTs.
// COPY uses larger parts but remains bounded by its per-column byte budget.
const PART_ROWS: usize = 65_536;
const STREAMED_PART_ROWS: usize = 262_144;

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

/// Estimate retained COPY row sizes once. A reusable scratch vector avoids
/// per-record allocations and repeated String length/accounting passes.
fn estimate_streamed_row(
    row: &[Option<String>],
    scratch: &mut [usize],
) -> Result<(usize, bool), String> {
    if row.len() != scratch.len() {
        return Err("COPY row estimate width mismatch".into());
    }
    let mut total = 0usize;
    let mut oversized_column = false;
    for (cell, size) in row.iter().zip(scratch.iter_mut()) {
        *size = cell
            .as_ref()
            .map_or(16, |value| value.len().saturating_add(16));
        total = total.checked_add(*size).ok_or("COPY record size overflow")?;
        oversized_column |= *size > MAX_STREAMED_COLUMN_BYTES;
    }
    Ok((total, oversized_column))
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
        for source in existing {
            verify_wide_part(source).map_err(|error| error.to_string())?;
            let name = source.file_name().ok_or("invalid stored part filename")?;
            let destination = staging.join(name);
            fs::create_dir(&destination)
                .map_err(|error| format!("cannot stage existing part: {error}"))?;
            for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                if !entry
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_file()
                {
                    return Err("cannot hard-link a non-regular part entry".into());
                }
                fs::hard_link(entry.path(), destination.join(entry.file_name()))
                    .map_err(|error| format!("cannot stage immutable part file: {error}"))?;
            }
            sync_directory(&destination)?;
        }
        let mut buffer: StoredRows = Vec::with_capacity(PART_ROWS.min(65_536));
        let mut buffered_bytes = 0usize;
        let mut column_bytes = vec![0usize; cols.len()];
        let mut row_size_scratch = vec![0usize; cols.len()];
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
            let (row_bytes, oversized_column) =
                estimate_streamed_row(&row, &mut row_size_scratch)?;
            if row_bytes > MAX_STREAMED_PART_BYTES || oversized_column {
                return Err("COPY row exceeds streaming part budget".into());
            }
            let column_overflow = row_size_scratch.iter().zip(&column_bytes).any(|(&size, &used)| {
                used.checked_add(size)
                    .is_none_or(|total| total > MAX_STREAMED_COLUMN_BYTES)
            });
            if !buffer.is_empty()
                && (buffer.len() >= STREAMED_PART_ROWS
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
            for (&size, used) in row_size_scratch.iter().zip(&mut column_bytes) {
                *used = used
                    .checked_add(size)
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
mod streaming_budget_single_pass_tests {
    use super::*;

    #[test]
    fn row_estimates_preserve_null_and_wide_column_budgets() {
        let values = vec![
            Some("3".into()),
            None,
            Some("hello".into()),
            Some("x".repeat(1000)),
        ];
        let mut scratch = vec![usize::MAX; values.len()];
        let (total, oversized) = estimate_streamed_row(&values, &mut scratch).unwrap();
        assert_eq!(scratch, vec![17, 16, 21, 1016]);
        assert_eq!(total, 1070);
        assert!(!oversized);
        let huge = vec![Some("x".repeat(MAX_STREAMED_COLUMN_BYTES))];
        let mut scratch = [0];
        let (size, oversized) = estimate_streamed_row(&huge, &mut scratch).unwrap();
        assert_eq!(size, MAX_STREAMED_COLUMN_BYTES + 16);
        assert!(oversized);
    }

    #[test]
    fn million_row_estimation_reuses_one_scratch_allocation() {
        let row = vec![Some("100000".into()), Some("EU".into()), None, Some("42".into())];
        let mut scratch = vec![0; row.len()];
        let pointer = scratch.as_ptr();
        let mut total = 0u64;
        for _ in 0..100_000 {
            let (bytes, oversized) = estimate_streamed_row(&row, &mut scratch).unwrap();
            assert!(!oversized);
            total += bytes as u64;
        }
        assert_eq!(pointer, scratch.as_ptr());
        assert_eq!(total, 100_000u64 * (22 + 18 + 16 + 18));
    }
}
