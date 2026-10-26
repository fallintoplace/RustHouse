use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-catalog-query-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    dir
}

fn sql(query: &str, catalog: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--catalog"])
        .arg(catalog)
        .output()
        .unwrap()
}

#[test]
fn empty_catalog_tables_have_select_headers_and_zero_counts() {
    let root = catalog();
    assert!(sql("CREATE TABLE events (id UInt64, name String)", &root)
        .status
        .success());
    let count = sql("SELECT count(*) FROM events", &root);
    assert!(count.status.success(), "{count:?}");
    assert_eq!(count.stdout, b"count\n0\n");
    let rows = sql("SELECT id, name FROM events", &root);
    assert!(rows.status.success(), "{rows:?}");
    assert_eq!(rows.stdout, b"id\tname\n");
    assert!(!root.join("events.parts").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_select_supports_filters_aggregates_and_saved_parts() {
    let root = catalog();
    assert!(sql("CREATE TABLE events (id Int64, name String, score Nullable(Int64))", &root)
        .status
        .success());
    for statement in [
        "INSERT INTO events VALUES (-3, 'alpha', NULL), (4, 'beta', 2)",
        "INSERT INTO events (id, name) VALUES (7, 'gamma')",
    ] {
        let output = sql(statement, &root);
        assert!(output.status.success(), "{output:?}");
    }
    let selected = sql(
        "SELECT id, name FROM events WHERE id >= 0 ORDER BY id DESC",
        &root,
    );
    assert!(selected.status.success(), "{selected:?}");
    assert_eq!(selected.stdout, b"id\tname\n7\tgamma\n4\tbeta\n");
    let count = sql("SELECT count(*) FROM events WHERE score IS NULL", &root);
    assert!(count.status.success(), "{count:?}");
    assert_eq!(count.stdout, b"count\n2\n");
    let explain = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT id FROM events", "--catalog"])
        .arg(&root)
        .arg("--explain")
        .output()
        .unwrap();
    assert!(explain.status.success(), "{explain:?}");
    assert!(String::from_utf8_lossy(&explain.stdout).contains("table events"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_select_rejects_missing_tables_and_ambiguous_paths() {
    let root = catalog();
    let missing = sql("SELECT * FROM missing", &root);
    assert!(!missing.status.success());
    let conflicting = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(2)", "--catalog"])
        .arg(&root)
        .args(["--part", "/tmp/invalid-part"])
        .output()
        .unwrap();
    assert!(!conflicting.status.success());
    let generated = sql("SELECT number FROM numbers(2)", &root);
    assert!(generated.status.success(), "{generated:?}");
    assert_eq!(generated.stdout, b"number\n0\n1\n");
    fs::remove_dir_all(root).unwrap();
}
