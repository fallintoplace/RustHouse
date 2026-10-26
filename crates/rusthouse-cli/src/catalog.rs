use rusthouse_query::{ColumnDef, Ddl, parse_ddl};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);
const MAX_SCHEMA_BYTES: u64 = 32 * 1024;

pub(super) fn apply(catalog: &Path, ddl: &Ddl) -> Result<(), String> {
    match ddl {
        Ddl::ShowTables => show_tables(catalog),
        Ddl::DescribeTable { name } => describe_table(catalog, name),
        Ddl::CreateTable {
            name,
            columns,
            if_not_exists,
        } => create_table(catalog, name, columns, *if_not_exists),
        Ddl::DropTable { name, if_exists } => drop_table(catalog, name, *if_exists),
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
        Ddl::RenameTable { from, to } => rename_table(catalog, from, to),
    }
}

fn describe_table(catalog: &Path, name: &str) -> Result<(), String> {
    let columns = read_schema(catalog, name)?;
    println!("name\ttype");
    for column in columns {
        println!("{}\t{}", column.name, column.data_type);
    }
    Ok(())
}

fn show_tables(catalog: &Path) -> Result<(), String> {
    let entries = match fs::read_dir(catalog) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("name");
            return Ok(());
        }
        Err(error) => return Err(format!("cannot list catalog {}: {error}", catalog.display())),
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
        if name.len() > 128
            || !matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            continue;
        }
        names.push(name.to_owned());
    }
    names.sort_unstable();
    println!("name");
    for name in names {
        println!("{name}");
    }
    Ok(())
}

fn drop_table(catalog: &Path, name: &str, if_exists: bool) -> Result<(), String> {
    let path = catalog.join(format!("{name}.schema"));
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
    println!("Dropped table {name} (schema only)");
    Ok(())
}

fn read_schema(catalog: &Path, name: &str) -> Result<Vec<ColumnDef>, String> {
    let path = catalog.join(format!("{name}.schema"));
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot read table {name}: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err(format!("not a regular schema file: {name}"));
    }
    if metadata.len() > MAX_SCHEMA_BYTES {
        return Err(format!("schema is too large for table {name}"));
    }
    let content =
        fs::read_to_string(&path).map_err(|error| format!("cannot read table {name}: {error}"))?;
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
        if columns.len() == 64
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

fn replace_schema(catalog: &Path, name: &str, columns: &[ColumnDef]) -> Result<(), String> {
    let path = catalog.join(format!("{name}.schema"));
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
        file.write_all(schema_contents(columns).as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &path)
    };
    if let Err(error) = update() {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot update schema for {name}: {error}"));
    }
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
    if columns.len() >= 64 {
        return Err("DDL supports at most 64 columns".into());
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

fn rename_table(catalog: &Path, from: &str, to: &str) -> Result<(), String> {
    if from == to {
        return Err("source and destination table names must differ".into());
    }
    // Validate the source before moving it. Only versioned catalog files may be renamed.
    read_schema(catalog, from)?;
    let source = catalog.join(format!("{from}.schema"));
    let destination = catalog.join(format!("{to}.schema"));
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
    let path = catalog.join(format!("{name}.schema"));
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && if_not_exists => {
            // An existing schema must be safe and readable before ignoring CREATE.
            read_schema(catalog, name)?;
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(format!("table already exists: {name}"));
        }
        Err(error) => {
            return Err(format!("cannot create table {name}: {error}"));
        }
    };
    if let Err(error) = file
        .write_all(schema_contents(columns).as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(format!("cannot write schema for {name}: {error}"));
    }
    println!("Created table {name} (schema only)");
    Ok(())
}
