use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-create-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn ddl(sql: &str, directory: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql, "--catalog"])
        .arg(directory)
        .output()
        .unwrap()
}

#[test]
fn persists_schema_without_creating_mergetree_parts() {
    let dir = catalog();
    let first = ddl("CREATE TABLE events (id UInt64, payload Nullable(String))", &dir);
    assert!(first.status.success(), "{first:?}");
    assert_eq!(
        fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\npayload\tNullable(String)\n"
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

    let again = ddl("CREATE TABLE events (id UInt64)", &dir);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("already exists"));

    let skipped = ddl("CREATE TABLE IF NOT EXISTS events (id UInt8)", &dir);
    assert!(skipped.status.success(), "{skipped:?}");
    assert_eq!(
        fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\npayload\tNullable(String)\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_unsafe_or_unsupported_statements_without_touching_disk() {
    let dir = catalog();
    for sql in [
        "CREATE TABLE test (id UInt64, id String)",
        "CREATE TABLE test (id UInt64) ENGINE = MergeTree",
        "CREATE TABLE test ()",
        "CREATE TABLE test (id UInt64); DROP TABLE test",
        "CREATE TABLE test (id Decimal(10))",
        "CREATE TABLE ../outside (id UInt64)",
    ] {
        let output = ddl(sql, &dir);
        assert!(!output.status.success(), "{sql}: {output:?}");
        assert!(!dir.exists());
    }

    let no_catalog = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "CREATE TABLE test (id UInt64)"])
        .output()
        .unwrap();
    assert!(!no_catalog.status.success());

    let illegal_part = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "CREATE TABLE test (id UInt64)", "--catalog"])
        .arg(&dir)
        .args(["--part", "real-data"])
        .output()
        .unwrap();
    assert!(!illegal_part.status.success());
    assert!(!dir.exists());

    let preview = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "CREATE TABLE test (id UInt64)", "--catalog"])
        .arg(&dir)
        .arg("--show-ast")
        .output()
        .unwrap();
    assert!(preview.status.success(), "{preview:?}");
    assert!(String::from_utf8_lossy(&preview.stdout).contains("CreateTable"));
    assert!(!dir.exists());
}
