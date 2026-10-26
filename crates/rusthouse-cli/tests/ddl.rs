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
