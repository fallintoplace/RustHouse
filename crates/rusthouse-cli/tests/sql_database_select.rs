use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-database-select-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(query: &str, dir: &Path, options: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(dir)
        .args(options)
        .output()
        .unwrap()
}

#[test]
fn reads_empty_and_persisted_named_tables() {
    let db = database();
    let created = sql("CREATE TABLE events (id UInt64, name String)", &db, &[]);
    assert!(created.status.success(), "{created:?}");
    let empty = sql("SELECT count(*) FROM events", &db, &[]);
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"count\n0\n");

    let inserted = sql(
        "INSERT INTO events VALUES (2, 'Grace'), (1, 'Ada')",
        &db,
        &[],
    );
    assert!(inserted.status.success(), "{inserted:?}");
    let result = sql(
        "SELECT id, name FROM events WHERE id >= 1 ORDER BY id",
        &db,
        &[],
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"id\tname\n1\tAda\n2\tGrace\n");

    let renamed = sql(
        "SELECT * RENAME name AS customer_name FROM events ORDER BY customer_name DESC",
        &db,
        &[],
    );
    assert!(renamed.status.success(), "{renamed:?}");
    assert_eq!(renamed.stdout, b"id\tcustomer_name\n2\tGrace\n1\tAda\n");
    let renamed_json = sql(
        "SELECT * RENAME name AS customer_name FROM events LIMIT 1",
        &db,
        &["--format", "json"],
    );
    assert!(renamed_json.status.success(), "{renamed_json:?}");
    assert!(String::from_utf8_lossy(&renamed_json.stdout).contains("\"customer_name\""));

    let count = sql("SELECT count(*) FROM events", &db, &[]);
    assert_eq!(count.stdout, b"count\n2\n");
    let capped = sql(
        "SELECT id FROM events ORDER BY id",
        &db,
        &["--max-rows", "1"],
    );
    assert_eq!(capped.stdout, b"id\n1\n");
    let json = sql(
        "SELECT name FROM events ORDER BY name",
        &db,
        &["--format", "json"],
    );
    assert!(json.status.success(), "{json:?}");
    assert!(String::from_utf8_lossy(&json.stdout).contains("Ada"));
    assert!(String::from_utf8_lossy(&json.stdout).contains("Grace"));
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn rejects_unknown_tables_and_incompatible_options() {
    let db = database();
    let unknown = sql("SELECT * FROM missing", &db, &[]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("cannot read table"));

    let conflicting = sql("SELECT 1", &db, &["--part", "somewhere"]);
    assert!(!conflicting.status.success());
    assert!(String::from_utf8_lossy(&conflicting.stderr).contains("cannot be combined"));

    let generated = sql("SELECT number FROM numbers(3)", &db, &[]);
    assert!(generated.status.success(), "{generated:?}");
    assert_eq!(generated.stdout, b"number\n0\n1\n2\n");
    assert!(!db.exists());
}
