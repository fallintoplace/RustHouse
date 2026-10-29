use rusthouse_query::{ColumnDef, Ddl, parse_ddl};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);
const MAX_SCHEMA_BYTES: u64 = 32 * 1024;
const MAX_CATALOG_IDENTIFIER_BYTES: usize = 128;
const MAX_CATALOG_COLUMNS: usize = 128;

fn schema_path(catalog: &Path, name: &str) -> PathBuf {
    catalog.join(format!("{name}.schema"))
}

fn sync_catalog_dir(catalog: &Path) -> Result<(), String> {
    fs::File::open(catalog)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("cannot sync catalog {}: {error}", catalog.display()))
}

pub(super) fn apply(catalog: &Path, ddl: &Ddl) -> Result<(), String> {
    match ddl {
        Ddl::ShowTables => show_tables(catalog, None, false),
        Ddl::ShowTablesLike { pattern } => show_tables(catalog, Some(pattern), false),
        Ddl::ShowTablesNotLike { pattern } => show_tables(catalog, Some(pattern), true),
        Ddl::ShowCreateTable { name } => show_create_table(catalog, name),
        Ddl::DescribeTable { name } => describe_table(catalog, name),
        Ddl::CreateTable {
            name,
            columns,
            if_not_exists,
        } => create_table(catalog, name, columns, *if_not_exists),
        Ddl::CreateTableLike {
            name,
            source,
            if_not_exists,
        } => {
            let source_columns = read_schema(catalog, source)?;
            create_table(catalog, name, &source_columns, *if_not_exists)
        }
        Ddl::DropTable { name, if_exists } => drop_table(catalog, name, *if_exists),
        Ddl::TruncateTable { .. } => Err("TRUNCATE requires --database".into()),
        Ddl::OptimizeTable { .. } => Err("OPTIMIZE requires --database".into()),
        Ddl::AddColumn {
            table,
            column,
            if_not_exists,
        } => add_column(catalog, table, column, *if_not_exists),
        Ddl::DropColumn {
            table,
            column,
            if_exists,
        } => drop_column(catalog, table, column, *if_exists),
        Ddl::RenameColumn { table, from, to } => rename_column(catalog, table, from, to),
        Ddl::ModifyColumn { table, column } => modify_column(catalog, table, column),
        Ddl::RenameTable { from, to } => rename_table(catalog, from, to),
    }
}

fn show_create_table(catalog: &Path, name: &str) -> Result<(), String> {
    let columns = read_schema(catalog, name)?;
    println!("CREATE TABLE {name} (");
    for (index, column) in columns.iter().enumerate() {
        let suffix = if index + 1 == columns.len() { "" } else { "," };
        println!("    {} {}{suffix}", column.name, column.data_type);
    }
    println!(");");
    Ok(())
}

fn describe_table(catalog: &Path, name: &str) -> Result<(), String> {
    let columns = read_schema(catalog, name)?;
    println!("name\ttype");
    for column in columns {
        println!("{}\t{}", column.name, column.data_type);
    }
    Ok(())
}

/// Match SQL LIKE over validated ASCII table names. % matches any number of
/// bytes, _ matches one byte, and backslash escapes either wildcard.
fn table_matches_like(name: &str, pattern: &str) -> bool {
    let name = name.as_bytes();
    let mut states = vec![false; name.len() + 1];
    states[0] = true;
    let mut pattern = pattern.bytes();
    while let Some(mut ch) = pattern.next() {
        let escaped = ch == b'\\';
        if escaped {
            let Some(next) = pattern.next() else {
                return false;
            };
            ch = next;
        }
        let mut next = vec![false; name.len() + 1];
        if !escaped && ch == b'%' {
            next[0] = states[0];
            for i in 1..=name.len() {
                next[i] = states[i] || next[i - 1];
            }
        } else {
            for i in 0..name.len() {
                if states[i] && ((!escaped && ch == b'_') || name[i] == ch) {
                    next[i + 1] = true;
                }
            }
        }
        states = next;
    }
    states[name.len()]
}

fn show_tables(catalog: &Path, pattern: Option<&str>, negated: bool) -> Result<(), String> {
    let entries = match fs::read_dir(catalog) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("name");
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "cannot list catalog {}: {error}",
                catalog.display()
            ));
        }
    };

    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read catalog entry: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("cannot inspect catalog entry: {error}"))?
            .is_file()
        {
            continue;
        }
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        let Some(name) = filename.strip_suffix(".schema") else {
            continue;
        };
        let mut bytes = name.bytes();
        if name.len() > MAX_CATALOG_IDENTIFIER_BYTES
            || !matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            continue;
        }
        if pattern.is_none_or(|pattern| table_matches_like(name, pattern) != negated) {
            names.push(name.to_owned());
        }
    }
    names.sort_unstable();
    println!("name");
    for name in names {
        println!("{name}");
    }
    Ok(())
}

fn drop_table(catalog: &Path, name: &str, if_exists: bool) -> Result<(), String> {
    let path = schema_path(catalog, name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && if_exists => {
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("unknown catalog table: {name}"));
        }
        Err(error) => return Err(format!("cannot inspect table {name}: {error}")),
    };
    if !metadata.file_type().is_file() {
        return Err(format!("not a regular schema file: {name}"));
    }
    fs::remove_file(&path).map_err(|error| format!("cannot drop table {name}: {error}"))?;
    sync_catalog_dir(catalog).map_err(|error| {
        format!("table {name} was dropped but durability is uncertain: {error}")
    })?;
    println!("Dropped table {name} (schema only)");
    Ok(())
}

pub(super) fn read_schema(catalog: &Path, name: &str) -> Result<Vec<ColumnDef>, String> {
    let path = schema_path(catalog, name);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot read table {name}: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err(format!("not a regular schema file: {name}"));
    }
    if metadata.len() > MAX_SCHEMA_BYTES {
        return Err(format!("schema is too large for table {name}"));
    }
    // Limit reads even if a schema grows between metadata inspection and open.
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|error| format!("cannot read table {name}: {error}"))?
        .take(MAX_SCHEMA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read table {name}: {error}"))?;
    if bytes.len() as u64 > MAX_SCHEMA_BYTES {
        return Err(format!("schema is too large for table {name}"));
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| format!("invalid UTF-8 in schema for table {name}"))?;
    let mut lines = content.lines();
    if lines.next() != Some("rusthouse-schema-v1") {
        return Err(format!("invalid schema version for table {name}"));
    }
    let mut columns = Vec::new();
    for line in lines {
        let (column_name, data_type) = line
            .split_once('\t')
            .ok_or_else(|| format!("corrupt schema for table {name}"))?;
        let sql = format!("CREATE TABLE _check ({column_name} {data_type})");
        if columns.len() == MAX_CATALOG_COLUMNS
            || !matches!(parse_ddl(&sql), Ok(Some(Ddl::CreateTable { .. })))
            || columns
                .iter()
                .any(|column: &ColumnDef| column.name == column_name)
        {
            return Err(format!("corrupt schema for table {name}"));
        }
        columns.push(ColumnDef {
            name: column_name.to_string(),
            data_type: data_type.to_string(),
        });
    }
    if columns.is_empty() || !content.ends_with('\n') {
        return Err(format!("corrupt schema for table {name}"));
    }
    Ok(columns)
}

fn schema_contents(columns: &[ColumnDef]) -> String {
    let mut schema = String::from("rusthouse-schema-v1\n");
    for column in columns {
        schema.push_str(&column.name);
        schema.push('\t');
        schema.push_str(&column.data_type);
        schema.push('\n');
    }
    schema
}

fn persist_schema(file: &mut fs::File, columns: &[ColumnDef]) -> std::io::Result<()> {
    file.write_all(schema_contents(columns).as_bytes())?;
    file.sync_all()
}

fn replace_schema(catalog: &Path, name: &str, columns: &[ColumnDef]) -> Result<(), String> {
    let path = schema_path(catalog, name);
    let temp = catalog.join(format!(
        ".{name}.schema.tmp.{}.{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let update = || -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        persist_schema(&mut file, columns)?;
        fs::rename(&temp, &path)
    };
    if let Err(error) = update() {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot update schema for {name}: {error}"));
    }
    // The replacement is visible now; sync its directory entry for durability.
    sync_catalog_dir(catalog)?;
    Ok(())
}

fn add_column(
    catalog: &Path,
    table: &str,
    column: &ColumnDef,
    if_not_exists: bool,
) -> Result<(), String> {
    let mut columns = read_schema(catalog, table)?;
    if columns.iter().any(|entry| entry.name == column.name) {
        if if_not_exists {
            return Ok(());
        }
        return Err(format!("column already exists: {}", column.name));
    }
    if columns.len() >= MAX_CATALOG_COLUMNS {
        return Err("DDL supports at most 128 columns".into());
    }
    columns.push(column.clone());
    replace_schema(catalog, table, &columns)?;
    println!("Added column {} to {table} (schema only)", column.name);
    Ok(())
}

fn drop_column(catalog: &Path, table: &str, column: &str, if_exists: bool) -> Result<(), String> {
    let mut columns = read_schema(catalog, table)?;
    let Some(index) = columns.iter().position(|entry| entry.name == column) else {
        if if_exists {
            return Ok(());
        }
        return Err(format!("unknown column: {column}"));
    };
    if columns.len() == 1 {
        return Err(format!("cannot drop the last column from table {table}"));
    }
    columns.remove(index);
    replace_schema(catalog, table, &columns)?;
    println!("Dropped column {column} from {table} (schema only)");
    Ok(())
}

fn modify_column(catalog: &Path, table: &str, column: &ColumnDef) -> Result<(), String> {
    let mut columns = read_schema(catalog, table)?;
    let index = columns
        .iter()
        .position(|entry| entry.name == column.name)
        .ok_or_else(|| format!("unknown column: {}", column.name))?;
    if columns[index].data_type == column.data_type {
        return Ok(());
    }
    columns[index].data_type.clone_from(&column.data_type);
    replace_schema(catalog, table, &columns)?;
    println!("Modified column {} in {table} (schema only)", column.name);
    Ok(())
}

fn rename_column(catalog: &Path, table: &str, from: &str, to: &str) -> Result<(), String> {
    if from == to {
        return Err("source and destination column names must differ".into());
    }
    let mut columns = read_schema(catalog, table)?;
    let index = columns
        .iter()
        .position(|entry| entry.name == from)
        .ok_or_else(|| format!("unknown column: {from}"))?;
    if columns.iter().any(|entry| entry.name == to) {
        return Err(format!("column already exists: {to}"));
    }
    columns[index].name = to.to_owned();
    replace_schema(catalog, table, &columns)?;
    println!("Renamed column {from} to {to} in {table} (schema only)");
    Ok(())
}

fn rename_table(catalog: &Path, from: &str, to: &str) -> Result<(), String> {
    if from == to {
        return Err("source and destination table names must differ".into());
    }
    // Validate the source before moving it. Only versioned catalog files may be renamed.
    read_schema(catalog, from)?;
    let source = schema_path(catalog, from);
    let destination = schema_path(catalog, to);
    // Hard-link creation fails if the destination exists, unlike rename on Unix.
    // Source removal comes second, so failures cannot silently overwrite a table.
    match fs::hard_link(&source, &destination) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(format!("table already exists: {to}"));
        }
        Err(error) => return Err(format!("cannot rename table {from}: {error}")),
    }
    if let Err(error) = fs::remove_file(&source) {
        let _ = fs::remove_file(&destination);
        return Err(format!("cannot finish renaming table {from}: {error}"));
    }
    sync_catalog_dir(catalog).map_err(|error| {
        format!("table {from} was renamed but durability is uncertain: {error}")
    })?;
    println!("Renamed table {from} to {to} (schema only)");
    Ok(())
}

fn create_table(
    catalog: &Path,
    name: &str,
    columns: &[ColumnDef],
    if_not_exists: bool,
) -> Result<(), String> {
    fs::create_dir_all(catalog)
        .map_err(|error| format!("cannot create catalog {}: {error}", catalog.display()))?;
    let path = schema_path(catalog, name);
    let temp = catalog.join(format!(
        ".{name}.schema.tmp.{}.{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    // Never expose an incomplete schema at its public table name. Write and
    // sync the staging file first, then link it without replacing an existing
    // table (CREATE IF NOT EXISTS must still validate existing schemas).
    let staged = (|| -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        persist_schema(&mut file, columns)
    })();
    if let Err(error) = staged {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot stage schema for {name}: {error}"));
    }
    match fs::hard_link(&temp, &path) {
        Ok(()) => {}
        Err(error) => {
            let _ = fs::remove_file(&temp);
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                if if_not_exists {
                    read_schema(catalog, name)?;
                    return Ok(());
                }
                return Err(format!("table already exists: {name}"));
            }
            return Err(format!("cannot publish schema for {name}: {error}"));
        }
    }

    // After hard_link succeeds the schema may already be observable. On a
    // sync failure, leave the complete public file in place for recovery.
    sync_catalog_dir(catalog).map_err(|error| {
        format!("schema for {name} was published but durability is uncertain: {error}")
    })?;
    fs::remove_file(&temp)
        .map_err(|error| format!("schema for {name} was published but cleanup failed: {error}"))?;
    sync_catalog_dir(catalog)?;
    println!("Created table {name} (schema only)");
    Ok(())
}

#[cfg(test)]
mod show_tables_like_tests {
    use super::table_matches_like;

    #[test]
    fn matches_percent_single_wildcard_and_literal_escapes() {
        assert!(table_matches_like("events_2026", "events%"));
        assert!(table_matches_like("orders_1", r"orders\_%"));
        assert!(!table_matches_like("ordersX1", r"orders\_%"));
        assert!(table_matches_like("xy", "x_"));
        assert!(!table_matches_like("xyz", "x_"));
        assert!(table_matches_like("any", "%"));
        assert!(!table_matches_like("any", "other%"));
        assert!(!table_matches_like("any", r"any\"));
    }
}
