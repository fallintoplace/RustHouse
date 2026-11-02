use super::*;

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

pub(crate) fn apply_ddl(database: &Path, ddl: &Ddl) -> Result<(), String> {
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
