use rusthouse_query::{ColumnDef, Ddl};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub(super) fn apply(catalog: &Path, ddl: &Ddl) -> Result<(), String> {
    match ddl {
        Ddl::CreateTable {
            name,
            columns,
            if_not_exists,
        } => create_table(catalog, name, columns, *if_not_exists),
        Ddl::DropTable { name, if_exists } => drop_table(catalog, name, *if_exists),
    }
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
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(format!("table already exists: {name}"));
        }
        Err(error) => {
            return Err(format!("cannot create table {name}: {error}"));
        }
    };
    let mut schema = String::from("rusthouse-schema-v1\n");
    for column in columns {
        schema.push_str(&column.name);
        schema.push('\t');
        schema.push_str(&column.data_type);
        schema.push('\n');
    }
    if let Err(error) = file
        .write_all(schema.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(format!("cannot write schema for {name}: {error}"));
    }
    println!("Created table {name} (schema only)");
    Ok(())
}
