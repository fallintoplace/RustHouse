//! Non-destructive offline verification for local databases.
//! Run with writers stopped. No lock file or other database metadata is created.
use crate::catalog;
use rusthouse_storage::{
    Column, inspect_query_source, query_parts, resolve_active_snapshot, verify_part_file_sizes,
    verify_wide_part,
};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

const MAX_ROOT_ENTRIES: usize = 20_000;

#[derive(Default)]
struct Report {
    checked_tables: usize,
    warnings: usize,
    errors: usize,
}

impl Report {
    fn error(&mut self, item: &str, message: impl std::fmt::Display) {
        println!("ERROR {item}: {message}");
        self.errors += 1;
    }

    fn warning(&mut self, item: &str, message: impl std::fmt::Display) {
        println!("WARN {item}: {message}");
        self.warnings += 1;
    }
}

fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    !name.is_empty()
        && name.len() <= 128
        && matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn report_staging_entries(
    report: &mut Report,
    table: &str,
    entries: fs::ReadDir,
    prefix: &str,
    read_error: &'static str,
    warning: &str,
) {
    for entry in entries {
        let Ok(entry) = entry else {
            report.error(table, read_error);
            continue;
        };
        let filename = entry.file_name();
        let name = filename.to_string_lossy();
        if name.starts_with(prefix) {
            report.warning(table, format!("{warning}: {name}"));
        }
    }
}

fn inspect_staging(report: &mut Report, root: &Path, table: &str) {
    let Ok(entries) = fs::read_dir(root) else {
        report.error(table, "cannot list table parts directory");
        return;
    };
    report_staging_entries(
        report,
        table,
        entries,
        ".rusthouse-active.tmp-",
        "cannot inspect table directory entry",
        "unfinished snapshot pointer",
    );
    let versions = root.join(".rusthouse-versions");
    match fs::symlink_metadata(&versions) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            report.error(table, "snapshot generations path is not a real directory");
        }
        Ok(_) => match fs::read_dir(&versions) {
            Ok(entries) => report_staging_entries(
                report,
                table,
                entries,
                ".pending-",
                "cannot inspect snapshot generation entry",
                "unpublished snapshot staging",
            ),
            Err(error) => report.error(table, format!("cannot list snapshot generations: {error}")),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => report.error(
            table,
            format!("cannot inspect snapshot generations: {error}"),
        ),
    }
}

fn inspect_table(report: &mut Report, database: &Path, name: &str, schema: bool, parts: bool) {
    report.checked_tables += 1;
    if !schema {
        report.error(
            name,
            "orphaned data directory without schema; preserve it for recovery",
        );
        return;
    }
    if !parts {
        report.error(name, "schema exists but its data directory is missing");
        return;
    }

    let definitions = match catalog::read_schema(database, name) {
        Ok(columns) => columns,
        Err(error) => {
            report.error(name, error);
            return;
        }
    };
    let expected: Vec<Column> = definitions
        .into_iter()
        .map(|column| Column {
            name: column.name,
            data_type: column.data_type,
        })
        .collect();

    let root = database.join(format!("{name}.parts"));
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            report.error(
                name,
                "data root must be a real directory, not a symlink or file",
            );
            return;
        }
        Err(error) => {
            report.error(name, format!("cannot inspect table parts: {error}"));
            return;
        }
    }

    let before = report.errors;
    inspect_staging(report, &root, name);
    let active = match resolve_active_snapshot(&root) {
        Ok(path) => path,
        Err(error) => {
            report.error(name, format!("cannot resolve active snapshot: {error}"));
            return;
        }
    };
    let metadata = match inspect_query_source(&active) {
        Ok(metadata) => metadata,
        Err(error) => {
            report.error(name, format!("invalid active snapshot: {error}"));
            return;
        }
    };
    if metadata.columns != expected {
        report.error(name, "active data schema differs from catalog schema");
        return;
    }
    let active_parts = match query_parts(&active) {
        Ok(parts) => parts,
        Err(error) => {
            report.error(name, format!("cannot enumerate active parts: {error}"));
            return;
        }
    };
    for part in &active_parts {
        if let Err(error) = verify_wide_part(part) {
            report.error(name, format!("invalid part {}: {error}", part.display()));
        }
        if let Err(error) = verify_part_file_sizes(part) {
            report.error(
                name,
                format!("invalid part manifest {}: {error}", part.display()),
            );
        }
    }
    if report.errors == before {
        println!(
            "OK {name}: {} rows, {} active parts, {} columns",
            metadata.rows,
            active_parts.len(),
            metadata.columns.len()
        );
    }
}

fn record_catalog_entry(
    name: &str,
    schemas: &mut BTreeSet<String>,
    parts: &mut BTreeSet<String>,
    report: &mut Report,
) {
    if name.starts_with('.')
        && (name.contains(".pending-") || name.contains(".schema.tmp.") || name.contains(".drop-"))
    {
        report.warning(
            "catalog",
            format!("unpublished or interrupted staging entry: {name}"),
        );
    }
    if let Some(table) = name.strip_suffix(".schema") {
        if valid_name(table) {
            schemas.insert(table.to_owned());
        } else if !name.starts_with('.') {
            report.error("catalog", format!("invalid schema filename: {name}"));
        }
    }
    if let Some(table) = name.strip_suffix(".parts") {
        if valid_name(table) {
            parts.insert(table.to_owned());
        } else if !name.starts_with('.') {
            report.error("catalog", format!("invalid data directory name: {name}"));
        }
    }
}

fn parse_doctor_directory(mut args: impl Iterator<Item = OsString>) -> Result<OsString, String> {
    let (Some(flag), Some(directory), None) = (args.next(), args.next(), args.next()) else {
        return Err("usage: rusthouse doctor --database <directory>".into());
    };
    if flag != "--database" {
        return Err("usage: rusthouse doctor --database <directory>".into());
    }
    Ok(directory)
}

/// Print a deterministic diagnostic report without altering on-disk data.
/// Does not acquire a lock because doing so could create the lock inode.
pub(super) fn run(args: impl Iterator<Item = OsString>) -> Result<(), String> {
    let directory = parse_doctor_directory(args)?;
    let database = Path::new(&directory);
    let kind = fs::symlink_metadata(database)
        .map_err(|error| format!("cannot inspect database {}: {error}", database.display()))?;
    if !kind.file_type().is_dir() {
        return Err("database must be a real directory (symlinks are not supported)".into());
    }
    println!("RustHouse doctor: {}", database.display());
    let mut schemas = BTreeSet::new();
    let mut parts = BTreeSet::new();
    let mut report = Report::default();
    let entries = fs::read_dir(database).map_err(|error| error.to_string())?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_ROOT_ENTRIES {
            return Err("database has too many root entries to inspect safely".into());
        }
        let entry = entry.map_err(|error| format!("cannot read database entry: {error}"))?;
        let filename = entry.file_name();
        let Some(name) = filename.to_str() else {
            report.warning("catalog", "non-UTF-8 root entry was skipped");
            continue;
        };
        record_catalog_entry(name, &mut schemas, &mut parts, &mut report);
    }
    let names = schemas.union(&parts).cloned().collect::<Vec<_>>();
    for name in names {
        inspect_table(
            &mut report,
            database,
            &name,
            schemas.contains(&name),
            parts.contains(&name),
        );
    }
    println!(
        "Summary: {} table(s), {} warning(s), {} error(s)",
        report.checked_tables, report.warnings, report.errors
    );
    println!(
        "Validation includes block checksums and manifest file sizes, not full manifest file hashes."
    );
    if report.errors > 0 {
        Err(format!(
            "doctor found {} error(s); no files were changed",
            report.errors
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_unpublished_entries_in_both_staging_directories() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-doctor-staging-{}",
            std::process::id()
        ));
        fs::create_dir_all(root.join(".rusthouse-versions")).unwrap();
        fs::write(root.join(".rusthouse-active.tmp-1"), b"").unwrap();
        fs::write(root.join(".rusthouse-versions/.pending-1"), b"").unwrap();

        let mut report = Report::default();
        inspect_staging(&mut report, &root, "test");
        assert_eq!(report.warnings, 2);
        assert_eq!(report.errors, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_classifier_keeps_valid_tables_and_reports_staging_entries() {
        let mut schemas = BTreeSet::new();
        let mut parts = BTreeSet::new();
        let mut report = Report::default();
        for name in [
            "logs.schema",
            "logs.parts",
            ".schema.tmp.17",
            "bad-name.schema",
        ] {
            record_catalog_entry(name, &mut schemas, &mut parts, &mut report);
        }
        assert!(schemas.contains("logs"));
        assert!(parts.contains("logs"));
        assert_eq!(schemas.len(), 1);
        assert_eq!(parts.len(), 1);
        assert_eq!(report.warnings, 1);
        assert_eq!(report.errors, 1);
    }
}
