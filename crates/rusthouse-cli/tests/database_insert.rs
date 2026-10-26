use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-database-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(statement: &str, dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(dir)
        .output()
        .unwrap()
}

fn snapshot_sql(statement: &str, dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--part"])
        .arg(dir.join("events.parts"))
        .output()
        .unwrap()
}

#[test]
fn inserts_recoverable_wide_parts_with_named_columns() {
    let dir = database();
    let create = sql("CREATE TABLE events (id UInt64, name String, note Nullable(String))", &dir);
    assert!(create.status.success(), "{create:?}");
    let initial = snapshot_sql("SELECT count(*) FROM part", &dir);
    assert_eq!(initial.stdout, b"count\n0\n");
    let inserted = sql(
        "INSERT INTO events (name, id, note) VALUES ('Alice', 1, NULL), ('Bob', 2, 'hello')",
        &dir,
    );
    assert!(inserted.status.success(), "{inserted:?}");
    let second = sql("INSERT INTO events VALUES (3, 'Carol', 'world')", &dir);
    assert!(second.status.success(), "{second:?}");
    let result = snapshot_sql("SELECT id, name FROM part ORDER BY id", &dir);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"id\tname\n1\tAlice\n2\tBob\n3\tCarol\n");
    let count = snapshot_sql("SELECT count(*) FROM part", &dir);
    assert_eq!(count.stdout, b"count\n3\n");
    let tables = sql("SHOW TABLES", &dir);
    assert!(tables.status.success(), "{tables:?}");
    assert_eq!(tables.stdout, b"name\nevents\n");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_insert_leaves_existing_rows_unchanged() {
    let dir = database();
    assert!(sql("CREATE TABLE events (id UInt8, name String)", &dir).status.success());
    assert!(sql("INSERT INTO events VALUES (1, 'ok')", &dir).status.success());
    for statement in [
        "INSERT INTO events VALUES (256, 'bad')",
        "INSERT INTO events VALUES (2, NULL)",
        "INSERT INTO events (wrong) VALUES (3)",
        "INSERT INTO events VALUES (2)",
        "INSERT INTO events VALUES (2, 'ok'); DROP TABLE events",
    ] {
        let result = sql(statement, &dir);
        assert!(!result.status.success(), "{statement}: {result:?}");
        let count = snapshot_sql("SELECT count(*) FROM part", &dir);
        assert_eq!(count.stdout, b"count\n1\n", "{statement}");
    }
    let alter = sql("ALTER TABLE events ADD COLUMN note String", &dir);
    assert!(!alter.status.success());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn database_rejects_unqueryable_column_types() {
    let dir = database();
    let invalid = sql("CREATE TABLE events (id Array(String))", &dir);
    assert!(!invalid.status.success(), "{invalid:?}");
    assert!(!dir.join("events.schema").exists());
    let valid = sql("CREATE TABLE events (id UInt64)", &dir);
    assert!(valid.status.success(), "{valid:?}");
    let again = sql("CREATE TABLE IF NOT EXISTS events (other String)", &dir);
    assert!(again.status.success(), "{again:?}");
    assert_eq!(fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\n");
    fs::remove_dir_all(dir).unwrap();
}
