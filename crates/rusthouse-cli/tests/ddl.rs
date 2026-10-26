use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_CATALOG: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-{}-{}",
        std::process::id(),
        NEXT_CATALOG.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(query: &str, dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--catalog"])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn create_table_persists_schema_and_handles_existing_tables() {
    let dir = catalog();
    let created = sql("CREATE TABLE events (id UInt64, value Int64)", &dir);
    assert!(created.status.success(), "{created:?}");
    assert_eq!(
        fs::read_to_string(dir.join("events.rh")).unwrap(),
        "RustHouseTableV1\nid UInt64\nvalue Int64\n"
    );

    let duplicate = sql("CREATE TABLE events (id UInt64)", &dir);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("cannot create table"));
    let ignored = sql("CREATE TABLE IF NOT EXISTS events (other UInt64)", &dir);
    assert!(ignored.status.success(), "{ignored:?}");
    assert_eq!(
        fs::read_to_string(dir.join("events.rh")).unwrap(),
        "RustHouseTableV1\nid UInt64\nvalue Int64\n"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn create_requires_catalog_and_rejects_unsupported_flags() {
    let missing = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "CREATE TABLE events (id UInt64)"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--catalog"));

    let dir = catalog();
    let unsupported = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "CREATE TABLE events (id UInt64)", "--catalog"])
        .arg(&dir)
        .args(["--max-rows", "3"])
        .output()
        .unwrap();
    assert!(!unsupported.status.success());
    assert!(!dir.exists());
}

#[test]
fn show_tables_lists_only_valid_catalog_entries_in_sorted_order() {
    let dir = catalog();
    assert!(sql("CREATE TABLE beta (id UInt64)", &dir).status.success());
    assert!(sql("CREATE TABLE alpha (id UInt64)", &dir).status.success());
    fs::write(dir.join("notes.txt"), "ignore").unwrap();
    fs::write(dir.join("bad-name.rh"), "ignore").unwrap();
    let shown = sql("SHOW TABLES;", &dir);
    assert!(shown.status.success(), "{shown:?}");
    assert_eq!(
        String::from_utf8(shown.stdout).unwrap(),
        "name\nalpha\nbeta\n"
    );
    fs::remove_dir_all(&dir).unwrap();

    let empty = sql("SHOW TABLES", &dir);
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(String::from_utf8(empty.stdout).unwrap(), "name\n");
}

#[test]
fn describe_table_shows_schema_and_rejects_corruption() {
    let dir = catalog();
    assert!(
        sql("CREATE TABLE readings (ts DateTime, value Float64)", &dir)
            .status
            .success()
    );
    let described = sql("DESCRIBE TABLE readings", &dir);
    assert!(described.status.success(), "{described:?}");
    assert_eq!(
        String::from_utf8(described.stdout).unwrap(),
        "name\ttype\nts\tDateTime\nvalue\tFloat64\n"
    );
    let missing = sql("DESCRIBE TABLE unknown", &dir);
    assert!(!missing.status.success());
    fs::write(dir.join("readings.rh"), "invalid schema").unwrap();
    let broken = sql("DESCRIBE TABLE readings", &dir);
    assert!(!broken.status.success());
    assert!(broken.stdout.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn drop_table_removes_only_the_named_schema() {
    let dir = catalog();
    assert!(sql("CREATE TABLE keep (id UInt64)", &dir).status.success());
    assert!(
        sql("CREATE TABLE remove (id UInt64)", &dir)
            .status
            .success()
    );
    let dropped = sql("DROP TABLE remove", &dir);
    assert!(dropped.status.success(), "{dropped:?}");
    assert!(!dir.join("remove.rh").exists());
    assert!(dir.join("keep.rh").is_file());
    let duplicate = sql("DROP TABLE remove", &dir);
    assert!(!duplicate.status.success());
    let ignored = sql("DROP TABLE IF EXISTS remove", &dir);
    assert!(ignored.status.success(), "{ignored:?}");
    let ignored_again = sql("DROP TABLE IF EXISTS remove", &dir);
    assert!(ignored_again.status.success(), "{ignored_again:?}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn alter_add_column_persists_new_column_and_rejects_duplicates() {
    let dir = catalog();
    assert!(
        sql("CREATE TABLE metrics (id UInt64)", &dir)
            .status
            .success()
    );
    let added = sql("ALTER TABLE metrics ADD COLUMN value Float64", &dir);
    assert!(added.status.success(), "{added:?}");
    let description = sql("DESCRIBE TABLE metrics", &dir);
    assert_eq!(
        String::from_utf8(description.stdout).unwrap(),
        "name\ttype\nid\tUInt64\nvalue\tFloat64\n"
    );
    let repeated = sql("ALTER TABLE metrics ADD COLUMN value Int64", &dir);
    assert!(!repeated.status.success());
    let ignored = sql(
        "ALTER TABLE metrics ADD COLUMN IF NOT EXISTS value Int64",
        &dir,
    );
    assert!(ignored.status.success(), "{ignored:?}");
    let description = sql("DESCRIBE TABLE metrics", &dir);
    assert_eq!(
        String::from_utf8(description.stdout).unwrap(),
        "name\ttype\nid\tUInt64\nvalue\tFloat64\n"
    );
    let unsupported = sql("ALTER TABLE metrics ADD COLUMN stuff Nullable", &dir);
    assert!(!unsupported.status.success());
    assert!(
        !dir.join(format!(".metrics.{}.tmp", std::process::id()))
            .exists()
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn alter_rejects_unknown_and_corrupt_schemas_without_creating_files() {
    let dir = catalog();
    let missing = sql("ALTER TABLE missing ADD COLUMN id UInt64", &dir);
    assert!(!missing.status.success());
    assert!(!dir.exists());
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("damaged.rh"), "wrong\n").unwrap();
    let invalid = sql("ALTER TABLE damaged ADD COLUMN value UInt64", &dir);
    assert!(!invalid.status.success());
    assert_eq!(
        fs::read_to_string(dir.join("damaged.rh")).unwrap(),
        "wrong\n"
    );
    fs::remove_dir_all(&dir).unwrap();
}
