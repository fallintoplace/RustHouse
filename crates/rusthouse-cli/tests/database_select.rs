use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-db-select-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(statement: &str, database: &Path, opts: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .args(opts)
        .output()
        .unwrap()
}

#[test]
fn create_insert_and_select_persist_across_processes() {
    let dir = database();
    let create = sql(
        "CREATE TABLE metrics (id UInt64, service String, status Nullable(UInt64))",
        &dir,
        &[],
    );
    assert!(create.status.success(), "{create:?}");
    let empty = sql("SELECT count(*) FROM metrics", &dir, &[]);
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"count\n0\n");

    let insert = sql(
        "INSERT INTO metrics VALUES (1, 'api', NULL), (2, 'db', 12), (3, 'api', 6)",
        &dir,
        &[],
    );
    assert!(insert.status.success(), "{insert:?}");
    let second = sql("INSERT INTO metrics VALUES (4, 'worker', 4)", &dir, &[]);
    assert!(second.status.success(), "{second:?}");

    let filtered = sql(
        "SELECT id, service FROM metrics WHERE service = 'api' ORDER BY id DESC LIMIT 2",
        &dir,
        &[],
    );
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(filtered.stdout, b"id\tservice\n3\tapi\n1\tapi\n");

    let counted = sql("SELECT count(status) FROM metrics", &dir, &[]);
    assert!(counted.status.success(), "{counted:?}");
    assert_eq!(counted.stdout, b"count(status)\n3\n");

    let aggregate = sql("SELECT sum(status) FROM metrics", &dir, &[]);
    assert!(aggregate.status.success(), "{aggregate:?}");
    assert_eq!(aggregate.stdout, b"sum(status)\n22\n");

    let csv = sql(
        "SELECT id, service FROM metrics ORDER BY id",
        &dir,
        &["--format", "csv"],
    );
    assert!(csv.status.success(), "{csv:?}");
    assert_eq!(
        csv.stdout,
        b"id,service\r\n1,api\r\n2,db\r\n3,api\r\n4,worker\r\n"
    );

    let explain = sql("SELECT id FROM metrics WHERE id > 1", &dir, &["--explain"]);
    assert!(explain.status.success(), "{explain:?}");
    assert!(
        String::from_utf8(explain.stdout)
            .unwrap()
            .contains("Source: table metrics")
    );

    let absent = sql("SELECT id FROM missing", &dir, &[]);
    assert!(!absent.status.success());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn empty_tables_and_select_flags_work_without_mutation() {
    let dir = database();
    assert!(
        sql("CREATE TABLE events (name String, score Int64)", &dir, &[])
            .status
            .success()
    );
    let empty = sql("SELECT name, score FROM events", &dir, &[]);
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"name\tscore\n");
    let inserted = sql("INSERT INTO events VALUES ('abc', -2)", &dir, &[]);
    assert!(inserted.status.success(), "{inserted:?}");
    let results = sql(
        "SELECT name, score FROM events",
        &dir,
        &["--format", "json"],
    );
    assert!(results.status.success(), "{results:?}");
    assert_eq!(results.stdout, b"[{\"name\":\"abc\",\"score\":\"-2\"}]\n");
    let capped = sql("SELECT name FROM events", &dir, &["--max-rows", "0"]);
    assert!(capped.status.success(), "{capped:?}");
    assert_eq!(capped.stdout, b"name\n");
    let denied = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT name FROM events", "--database"])
        .arg(&dir)
        .args(["--part", "other"])
        .output()
        .unwrap();
    assert!(!denied.status.success());
    fs::remove_dir_all(dir).unwrap();
}
