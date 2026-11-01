//! Single-writer local database mode.
//! Tables use existing .schema metadata and immutable, atomically published wide parts.
use crate::catalog;
mod rewrite;
use rusthouse_query::{Ddl, Insert, RowPredicate, Select, Source, Update, UpdateValue};
use rusthouse_storage::{
    Column, DatabaseLock, acquire_database_lock, query_parts_with_metadata,
    resolve_active_snapshot, verify_wide_part, write_wide_part,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_STAGE: AtomicUsize = AtomicUsize::new(0);

// Debug-only subprocess fault points for commit boundary tests.
#[cfg(debug_assertions)]
pub(super) fn crash_if_requested(point: &str) {
    if std::env::var("RUSTHOUSE_DEBUG_CRASH_AT").is_ok_and(|value| value == point) {
        eprintln!("RustHouse debug crash point: {point}");
        std::process::exit(86);
    }
}

#[cfg(not(debug_assertions))]
pub(super) fn crash_if_requested(_point: &str) {}

fn lock(database: &Path) -> Result<DatabaseLock, String> {
    acquire_database_lock(database, true)
}

pub(super) fn read_lock(database: &Path) -> Result<DatabaseLock, String> {
    acquire_database_lock(database, false)
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
    let active = active_source(database, name)?;
    let (parts, source) = query_parts_with_metadata(&active).map_err(|error| error.to_string())?;
    if source.columns != cols {
        return Err(format!("table {name} schema differs from its stored data"));
    }
    Ok((active, parts))
}

fn active_source(database: &Path, name: &str) -> Result<PathBuf, String> {
    resolve_active_snapshot(&data_path(database, name)).map_err(|error| error.to_string())
}

/// Resolve a named SQL table to its immutable on-disk parts.
pub(super) fn select_source(database: &Path, table: &str) -> Result<PathBuf, String> {
    rusthouse_storage::resolve_database_table(database, table)
}

fn existing_path(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn sync_database_directory(database: &Path) -> Result<(), String> {
    fs::File::open(database)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "cannot sync database directory {}: {error}",
                database.display()
            )
        })
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
    // Persist the hidden schema before unlinking part data. Otherwise a
    // power loss can restore a visible schema after its files were deleted.
    sync_database_directory(database)?;
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
    validated_parts(database, from, &cols)?;
    // Move the complete table root, including its snapshot pointer and all generations.
    let source_parts = data_path(database, from);
    let source_schema = database.join(format!("{from}.schema"));
    let destination_schema = database.join(format!("{to}.schema"));
    let destination_parts = data_path(database, to);
    if existing_path(&destination_schema)? || existing_path(&destination_parts)? {
        return Err(format!("table already exists or has orphaned data: {to}"));
    }

    // Both the schema and parts have to move, so RENAME is not a single
    // filesystem operation. Link the destination schema *before* moving the
    // parts: before the move the source remains readable, and afterwards the
    // destination is readable. A crash can leave an extra schema alias, but
    // never a window where neither name has both schema and data.
    fs::hard_link(&source_schema, &destination_schema)
        .map_err(|error| format!("cannot stage destination schema {to}: {error}"))?;
    if let Err(error) = sync_database_directory(database) {
        let rollback = fs::remove_file(&destination_schema)
            .and_then(|_| fs::File::open(database).and_then(|dir| dir.sync_all()));
        if let Err(rollback) = rollback {
            return Err(format!(
                "cannot sync staged rename {from} to {to}: {error}; schema rollback failed: {rollback}"
            ));
        }
        return Err(format!("cannot sync staged rename {from} to {to}: {error}"));
    }
    if let Err(error) = fs::rename(&source_parts, &destination_parts) {
        let rollback = fs::remove_file(&destination_schema)
            .and_then(|_| fs::File::open(database).and_then(|dir| dir.sync_all()));
        if let Err(rollback) = rollback {
            return Err(format!(
                "cannot move table data from {from}: {error}; schema rollback failed: {rollback}"
            ));
        }
        return Err(format!("cannot move table data from {from}: {error}"));
    }
    // Do not remove the old schema until the new name is durably discoverable.
    sync_database_directory(database)
        .map_err(|error| format!("table data moved to {to}, but old schema remains: {error}"))?;
    fs::remove_file(&source_schema)
        .map_err(|error| format!("table {to} is readable but cannot remove old schema: {error}"))?;
    sync_database_directory(database)
        .map_err(|error| format!("table {to} was renamed but final sync failed: {error}"))?;
    println!("Renamed table {from} to {to}");
    Ok(())
}

/// Publish a new empty snapshot without temporarily removing the active table.
fn truncate_table(database: &Path, name: &str) -> Result<(), String> {
    let _lock = lock(database)?;
    let cols = columns(database, name)?;
    validated_parts(database, name, &cols)?;
    rewrite::publish_snapshot(database, name, &cols, &[])?;
    println!("Truncated table {name}");
    Ok(())
}

fn create_table_locked(
    database: &Path,
    name: &str,
    definitions: &[rusthouse_query::ColumnDef],
    if_not_exists: bool,
) -> Result<(), String> {
    let schema = database.join(format!("{name}.schema"));
    if existing_path(&schema)? {
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
    let cols: Vec<Column> = definitions
        .iter()
        .map(|column| Column {
            name: column.name.clone(),
            data_type: column.data_type.clone(),
        })
        .collect();
    let ddl = Ddl::CreateTable {
        name: name.to_owned(),
        columns: definitions.to_vec(),
        if_not_exists,
    };
    let result = (|| -> Result<(), String> {
        write_wide_part(&parts.join("all_0_0_0"), &cols, &[]).map_err(|error| error.to_string())?;
        // Persist the parts directory and its parent before the schema name
        // becomes visible. A published schema must not point at missing data
        // after a crash.
        fs::File::open(&parts)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("cannot sync new table parts: {error}"))?;
        fs::File::open(database)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("cannot sync new table directory: {error}"))?;
        catalog::apply(database, &ddl)
    })();
    if result.is_err() && matches!(existing_path(&schema), Ok(false)) {
        // A schema may have been published before its final sync failed.
        // Never remove its data in that case.
        let _ = fs::remove_dir_all(&parts);
    }
    result
}

pub(super) fn apply_ddl(database: &Path, ddl: &Ddl) -> Result<(), String> {
    match ddl {
        Ddl::ShowTables
        | Ddl::ShowTablesLike { .. }
        | Ddl::ShowTablesNotLike { .. }
        | Ddl::ShowCreateTable { .. }
        | Ddl::DescribeTable { .. } => {
            // Do not create a new database just to list empty tables.
            if database.is_dir() {
                let _read_lock = read_lock(database)?;
                catalog::apply(database, ddl)
            } else {
                catalog::apply(database, ddl)
            }
        }
        Ddl::CreateTable {
            name,
            columns: definitions,
            if_not_exists,
        } => {
            let _lock = lock(database)?;
            create_table_locked(database, name, definitions, *if_not_exists)
        }
        Ddl::CreateTableLike {
            name,
            source,
            if_not_exists,
        } => {
            let _lock = lock(database)?;
            let source_columns = catalog::read_schema(database, source)?;
            let physical_columns: Vec<Column> = source_columns
                .iter()
                .map(|column| Column {
                    name: column.name.clone(),
                    data_type: column.data_type.clone(),
                })
                .collect();
            validated_parts(database, source, &physical_columns)?;
            create_table_locked(database, name, &source_columns, *if_not_exists)
        }
        Ddl::DropTable { name, if_exists } => drop_table(database, name, *if_exists),
        Ddl::TruncateTable { name } => truncate_table(database, name),
        Ddl::OptimizeTable { name } => rewrite::compact_table(database, name),
        Ddl::RenameTable { from, to } => rename_table(database, from, to),
        _ => Err("database mode does not support ALTER yet".into()),
    }
}

/// Calculate one integer row update without narrowing UInt128 or wrapping.
fn checked_update_offset(
    current: &str,
    delta: u128,
    subtract: bool,
    signed: bool,
) -> Result<String, String> {
    if signed {
        let current = current
            .parse::<i128>()
            .map_err(|_| "invalid signed UPDATE column value")?;
        // A full UInt128 offset can still produce a valid signed result
        // when applied to a negative value. Do not narrow before arithmetic.
        let result = if subtract {
            current.checked_sub_unsigned(delta)
        } else {
            current.checked_add_unsigned(delta)
        };
        result
            .map(|value| value.to_string())
            .ok_or_else(|| "signed UPDATE arithmetic overflow".into())
    } else {
        let current = current
            .parse::<u128>()
            .map_err(|_| "invalid unsigned UPDATE column value")?;
        let result = if subtract {
            current.checked_sub(delta)
        } else {
            current.checked_add(delta)
        };
        result
            .map(|value| value.to_string())
            .ok_or_else(|| "unsigned UPDATE arithmetic overflow".into())
    }
}

enum PreparedUpdate {
    Literal {
        index: usize,
        value: Option<String>,
    },
    Offset {
        index: usize,
        delta: u128,
        subtract: bool,
        signed: bool,
    },
}

pub(super) fn update(database: &Path, update: &Update) -> Result<(), String> {
    let _lock = lock(database)?;
    let cols = columns(database, &update.table)?;
    let filter = RowPredicate::new(&update.select, &cols).map_err(|error| error.to_string())?;
    let assignments = update
        .assignments
        .iter()
        .map(|(name, value)| {
            let index = cols
                .iter()
                .position(|column| column.name == *name)
                .ok_or_else(|| format!("unknown UPDATE column: {name}"))?;
            match value {
                UpdateValue::Literal(value) => {
                    if value.is_none() && !cols[index].data_type.starts_with("Nullable(") {
                        return Err(format!("NULL is not allowed for UPDATE column: {name}"));
                    }
                    Ok(PreparedUpdate::Literal {
                        index,
                        value: value.clone(),
                    })
                }
                UpdateValue::Offset { delta, subtract } => {
                    let data_type = &cols[index].data_type;
                    let kind = data_type
                        .strip_prefix("Nullable(")
                        .and_then(|value| value.strip_suffix(')'))
                        .unwrap_or(data_type);
                    let signed = matches!(kind, "Int8" | "Int16" | "Int32" | "Int64" | "Int128");
                    let unsigned =
                        matches!(kind, "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128");
                    if !signed && !unsigned {
                        return Err(format!(
                            "UPDATE integer offset requires an integer column: {name}"
                        ));
                    }
                    Ok(PreparedUpdate::Offset {
                        index,
                        delta: *delta,
                        subtract: *subtract,
                        signed,
                    })
                }
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut changed = 0usize;
    rewrite::rewrite_rows_locked(database, &update.table, |batch, row, values| {
        if filter.matches(batch, row) {
            for assignment in &assignments {
                match assignment {
                    PreparedUpdate::Literal { index, value } => values[*index] = value.clone(),
                    PreparedUpdate::Offset {
                        index,
                        delta,
                        subtract,
                        signed,
                    } => {
                        // SQL NULL arithmetic stays NULL; copying a row uses the
                        // original column value, never an updated sibling.
                        if let Some(current) = values[*index].as_deref() {
                            values[*index] =
                                Some(checked_update_offset(current, *delta, *subtract, *signed)?);
                        }
                    }
                }
            }
            changed += 1;
        }
        Ok(true)
    })?;
    println!("Updated {changed} rows in {}", update.table);
    Ok(())
}

/// Delete matching rows by switching a complete immutable snapshot.
pub(super) fn delete(database: &Path, select: &Select) -> Result<(), String> {
    let _lock = lock(database)?;
    let Some(Source::Table(name)) = select.source.as_ref() else {
        return Err("DELETE requires a stored database table".into());
    };
    let cols = columns(database, name)?;
    let filter = RowPredicate::new(select, &cols).map_err(|error| error.to_string())?;
    let mut removed = 0usize;
    let (scanned, retained) = rewrite::rewrite_rows_locked(database, name, |batch, row, _| {
        if filter.matches(batch, row) {
            removed += 1;
            Ok(false)
        } else {
            Ok(true)
        }
    })?;
    if scanned != removed + retained {
        return Err("DELETE row count mismatch".into());
    }
    println!("Deleted {removed} rows from {name}");
    Ok(())
}

/// Run a bounded statement while holding the database's exclusive lock.
pub(super) fn with_write_lock<T>(
    database: &Path,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _lock = lock(database)?;
    operation()
}

pub(super) fn insert(database: &Path, insert: &Insert) -> Result<(), String> {
    with_write_lock(database, || insert_locked(database, insert))
}

/// Avoid a second Vec allocation on every COPY row when the source already
/// follows the table schema. Column-remapped and partial inserts keep their
/// existing null-fill semantics.
fn order_stream_row(
    source: Vec<Option<String>>,
    indices: &[usize],
    width: usize,
    identical_layout: bool,
) -> Vec<Option<String>> {
    if identical_layout {
        return source;
    }
    let mut values = vec![None; width];
    for (value, &index) in source.into_iter().zip(indices) {
        values[index] = value;
    }
    values
}

/// Validate and append an input stream with one atomic snapshot publication.
pub(super) fn insert_stream(
    database: &Path,
    table: &str,
    requested_columns: Option<Vec<String>>,
    next_row: impl FnMut() -> Result<Option<Vec<Option<String>>>, String>,
) -> Result<(), String> {
    with_write_lock(database, || {
        insert_stream_locked(database, table, requested_columns, next_row)
    })
}

/// Caller holds the database write lock while reading the source table.
/// The snapshot is published only after the complete stream validates.
pub(super) fn insert_stream_locked(
    database: &Path,
    table: &str,
    requested_columns: Option<Vec<String>>,
    mut next_row: impl FnMut() -> Result<Option<Vec<Option<String>>>, String>,
) -> Result<(), String> {
    let cols = columns(database, table)?;
    let (_, parts) = validated_parts(database, table, &cols)?;
    let indices = match requested_columns {
        Some(names) => names
            .iter()
            .map(|name| {
                cols.iter()
                    .position(|col| col.name == *name)
                    .ok_or_else(|| format!("unknown INSERT column: {name}"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => (0..cols.len()).collect::<Vec<_>>(),
    };
    // Empty imports should not create a new generation.
    let mut first = next_row()?;
    if first.is_none() {
        println!("Inserted 0 rows into {table}");
        return Ok(());
    }
    let identical_layout = indices.len() == cols.len() && indices.iter().copied().eq(0..cols.len());
    let count = rewrite::append_parts_stream(database, table, &cols, &parts, || {
        let next = match first.take() {
            Some(row) => Some(row),
            None => next_row()?,
        };
        let Some(source) = next else {
            return Ok(None);
        };
        if source.len() != indices.len() {
            return Err("INSERT row width does not match table schema".into());
        }
        let values = order_stream_row(source, &indices, cols.len(), identical_layout);
        for (column, value) in cols.iter().zip(&values) {
            if value.is_none() && !column.data_type.starts_with("Nullable(") {
                return Err(format!("missing or NULL value for {}", column.name));
            }
        }
        Ok(Some(values))
    })?;
    println!("Inserted {count} rows into {table}");
    Ok(())
}

/// The caller must already own the exclusive database write lock.
pub(super) fn insert_locked(database: &Path, insert: &Insert) -> Result<(), String> {
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
    if insert.rows.is_empty() {
        println!("Inserted 0 rows into {}", insert.table);
        return Ok(());
    }
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
    if parts.len() >= 16_384 {
        return Err("database table has reached its 16384-part limit".into());
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
        crash_if_requested("insert-after-staging");
        fs::rename(&staging, &destination)
            .map_err(|error| format!("cannot publish inserted rows: {error}"))?;
        crash_if_requested("insert-after-rename");
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

#[cfg(all(test, unix))]
mod lock_tests {
    use super::*;

    #[test]
    fn stale_lock_is_reusable_and_readers_block_writers() {
        let directory = std::env::temp_dir().join(format!(
            "rusthouse-advisory-lock-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(".rusthouse-write.lock");
        fs::write(&path, b"left behind by a terminated writer").unwrap();

        let writer = lock(&directory).unwrap();
        assert!(lock(&directory).is_err());
        assert!(read_lock(&directory).is_err());
        drop(writer);

        let first_reader = read_lock(&directory).unwrap();
        let second_reader = read_lock(&directory).unwrap();
        assert!(lock(&directory).is_err());
        drop(first_reader);
        assert!(lock(&directory).is_err());
        drop(second_reader);

        let writer = lock(&directory).unwrap();
        drop(writer);
        assert!(path.is_file(), "lock inode must stay available for reuse");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_symlinked_lock_file() {
        use std::os::unix::fs::symlink;

        let directory = std::env::temp_dir().join(format!(
            "rusthouse-advisory-symlink-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("target");
        fs::write(&target, b"untouched").unwrap();
        symlink(&target, directory.join(".rusthouse-write.lock")).unwrap();
        assert!(lock(&directory).is_err());
        assert_eq!(fs::read(target).unwrap(), b"untouched");
        fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod large_copy_row_layout_tests {
    use super::*;

    #[test]
    fn matching_schema_preserves_input_allocation_for_streamed_billions() {
        let source = vec![Some("1".into()), Some("west".into()), Some("2500".into())];
        let ptr = source.as_ptr();
        let direct = order_stream_row(source, &[0, 1, 2], 3, true);
        assert_eq!(ptr, direct.as_ptr());
        assert_eq!(direct[1].as_deref(), Some("west"));
    }

    #[test]
    fn mapped_and_partial_streamed_records_still_fill_missing_columns() {
        let source = vec![Some("west".into()), Some("77".into())];
        let result = order_stream_row(source, &[2, 0], 4, false);
        assert_eq!(
            result,
            [Some("77".into()), None, Some("west".into()), None,]
        );
        let source = vec![Some("replaced".into()), Some("final".into())];
        let result = order_stream_row(source, &[1, 1], 3, false);
        assert_eq!(result, [None, Some("final".into()), None]);
    }
}

#[cfg(all(test, unix))]
mod mutation_lock_order_tests {
    use super::*;

    #[test]
    fn update_acquires_lock_before_reading_table_schema() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-update-lock-order-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let held = lock(&dir).unwrap();
        let update_statement = rusthouse_query::parse_update("UPDATE missing SET id = 1")
            .unwrap()
            .unwrap();
        let error = update(&dir, &update_statement).unwrap_err();
        assert!(error.contains("acquire database write lock"), "{error}");
        drop(held);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod checked_update_arithmetic_tests {
    use super::*;

    #[test]
    fn exact_signed_and_unsigned_integer_offsets() {
        assert_eq!(checked_update_offset("7", 3, false, true).unwrap(), "10");
        assert_eq!(checked_update_offset("-7", 3, true, true).unwrap(), "-10");
        assert_eq!(checked_update_offset("7", 3, true, false).unwrap(), "4");
        assert_eq!(
            checked_update_offset(&(u128::MAX - 1).to_string(), 1, false, false).unwrap(),
            u128::MAX.to_string()
        );
        assert_eq!(
            checked_update_offset(&(i128::MIN + 1).to_string(), 1, true, true).unwrap(),
            i128::MIN.to_string()
        );
    }

    #[test]
    fn rejects_underflow_overflow_and_invalid_input() {
        assert!(checked_update_offset("0", 1, true, false).is_err());
        assert!(checked_update_offset(&u128::MAX.to_string(), 1, false, false).is_err());
        assert!(checked_update_offset(&i128::MIN.to_string(), 1, true, true).is_err());
        assert!(checked_update_offset(&i128::MAX.to_string(), 1, false, true).is_err());
        assert!(checked_update_offset("not-an-integer", 2, true, true).is_err());
        assert!(checked_update_offset("1.5", 2, false, true).is_err());
        assert!(checked_update_offset("1", u128::MAX, false, true).is_err());
    }
}

#[cfg(test)]
mod full_width_signed_update_offset_tests {
    use super::*;

    #[test]
    fn accepts_unsigned_offsets_that_cancel_negative_int128_values() {
        assert_eq!(
            checked_update_offset(&i128::MIN.to_string(), u128::MAX, false, true).unwrap(),
            i128::MAX.to_string(),
        );
        assert_eq!(
            checked_update_offset(&i128::MAX.to_string(), u128::MAX, true, true).unwrap(),
            i128::MIN.to_string(),
        );
        assert_eq!(
            checked_update_offset("0", 1u128 << 127, true, true).unwrap(),
            i128::MIN.to_string(),
        );
    }

    #[test]
    fn full_width_signed_offsets_still_fail_on_real_overflow() {
        assert!(checked_update_offset("0", 1u128 << 127, false, true).is_err());
        assert!(checked_update_offset("-1", u128::MAX, true, true).is_err());
        assert!(checked_update_offset(&i128::MAX.to_string(), 1, false, true).is_err());
    }
}
