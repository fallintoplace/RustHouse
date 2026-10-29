//! Shared, read-only table resolution for CLI and HTTP database queries.
//! A table is visible only if its schema and active immutable parts agree.

use crate::{inspect_query_source, resolve_active_snapshot};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_SCHEMA_BYTES: usize = 32 * 1024;

/// Resolve only catalog-visible, on-disk tables. Never expose orphaned
/// data directories or symlinks that the local database would not accept.
pub fn resolve_database_table(database: &Path, table: &str) -> Result<PathBuf, String> {
    let mut chars = table.bytes();
    if table.len() > 128
        || !matches!(chars.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        || !chars.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("invalid database table name".into());
    }
    let schema = database.join(format!("{table}.schema"));
    let schema_metadata = fs::symlink_metadata(&schema)
        .map_err(|error| format!("cannot read database table {table}: {error}"))?;
    if !schema_metadata.file_type().is_file() || schema_metadata.len() > MAX_SCHEMA_BYTES as u64 {
        return Err(format!("invalid database schema for {table}"));
    }
    let mut content = Vec::new();
    let file = fs::File::open(&schema)
        .map_err(|error| format!("cannot read database table {table}: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("cannot inspect opened schema for {table}: {error}"))?;
    if !opened.file_type().is_file() || opened.len() > MAX_SCHEMA_BYTES as u64 {
        return Err(format!("database schema changed before read for {table}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if schema_metadata.dev() != opened.dev() || schema_metadata.ino() != opened.ino() {
            return Err(format!("database schema replaced before read for {table}"));
        }
    }
    file.take(MAX_SCHEMA_BYTES as u64 + 1)
        .read_to_end(&mut content)
        .map_err(|error| format!("cannot read database table {table}: {error}"))?;
    if content.len() > MAX_SCHEMA_BYTES {
        return Err(format!("database schema exceeds size limit for {table}"));
    }
    let content = String::from_utf8(content)
        .map_err(|_| format!("invalid UTF-8 in database schema for {table}"))?;
    if !content.ends_with('\n') {
        return Err(format!("invalid database schema for {table}"));
    }
    let mut lines = content.lines();
    if lines.next() != Some("rusthouse-schema-v1") {
        return Err(format!("invalid database schema version for {table}"));
    }
    let expected: Vec<(&str, &str)> = lines
        .map(|line| {
            line.split_once('\t')
                .ok_or_else(|| format!("invalid database column in {table}"))
        })
        .collect::<Result<_, _>>()?;
    if expected.is_empty() || expected.len() > 128 {
        return Err(format!("invalid database columns for {table}"));
    }
    let parts = database.join(format!("{table}.parts"));
    if !fs::symlink_metadata(&parts)
        .map_err(|error| format!("cannot inspect data for {table}: {error}"))?
        .file_type()
        .is_dir()
    {
        return Err(format!(
            "database parts are not a real directory for {table}"
        ));
    }
    let active = resolve_active_snapshot(&parts).map_err(|error| error.to_string())?;
    let stored = inspect_query_source(&active).map_err(|error| error.to_string())?;
    if stored.columns.len() != expected.len()
        || !stored
            .columns
            .iter()
            .zip(expected)
            .all(|(stored, (name, kind))| stored.name == name && stored.data_type == kind)
    {
        return Err(format!("schema and stored data disagree for {table}"));
    }
    Ok(active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_table_names_before_filesystem_access() {
        for name in ["", "1table", "../table", "a/b", "one-two", "a.b"] {
            assert_eq!(
                resolve_database_table(Path::new("nonexistent-database"), name).unwrap_err(),
                "invalid database table name"
            );
        }
    }
}

#[cfg(all(test, unix))]
mod schema_identity_tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn separate_same_size_schema_files_have_distinct_inodes() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-schema-identity-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let a = root.join("a");
        let b = root.join("b");
        fs::write(&a, "schema1").unwrap();
        fs::write(&b, "schema2").unwrap();
        let first = fs::symlink_metadata(&a).unwrap();
        let second = fs::File::open(&b).unwrap().metadata().unwrap();
        assert_eq!(first.len(), second.len());
        assert_ne!(first.ino(), second.ino());
        fs::remove_dir_all(root).unwrap();
    }
}
