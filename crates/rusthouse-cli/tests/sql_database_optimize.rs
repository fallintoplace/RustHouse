use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-snapshot-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(query: &str, dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn optimize_preserves_rows_and_allows_new_inserts() {
    let dir = dir();
    assert!(
        sql(
            "CREATE TABLE t (id Int64, name String, note Nullable(String))",
            &dir
        )
        .status
        .success()
    );
    assert!(
        sql(
            "INSERT INTO t VALUES (1, 'first', NULL), (-2, 'second', 'hello')",
            &dir
        )
        .status
        .success()
    );
    assert!(
        sql("INSERT INTO t VALUES (3, 'third', 'world')", &dir)
            .status
            .success()
    );
    let before = sql("SELECT id, name FROM t ORDER BY id", &dir);
    assert!(before.status.success(), "{before:?}");
    let optimize = sql("OPTIMIZE TABLE t", &dir);
    assert!(optimize.status.success(), "{optimize:?}");
    let after = sql("SELECT id, name FROM t ORDER BY id", &dir);
    assert!(after.status.success(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    let pointer = fs::read_to_string(dir.join("t.parts/.rusthouse-active")).unwrap();
    assert_eq!(pointer, "g1\n");
    assert!(
        sql("INSERT INTO t VALUES (5, 'after', NULL)", &dir)
            .status
            .success()
    );
    assert_eq!(sql("SELECT count(*) FROM t", &dir).stdout, b"count\n4\n");
    let truncate = sql("TRUNCATE TABLE t", &dir);
    assert!(truncate.status.success(), "{truncate:?}");
    assert_eq!(sql("SELECT count(*) FROM t", &dir).stdout, b"count\n0\n");
    assert!(
        sql("INSERT INTO t VALUES (7, 'next', NULL)", &dir)
            .status
            .success()
    );
    assert_eq!(sql("SELECT count(*) FROM t", &dir).stdout, b"count\n1\n");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_snapshot_pointer_fails_closed() {
    let dir = dir();
    assert!(sql("CREATE TABLE t (id UInt64)", &dir).status.success());
    fs::write(dir.join("t.parts/.rusthouse-active"), "../unsafe\n").unwrap();
    let result = sql("SELECT id FROM t", &dir);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid database snapshot"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn optimize_requires_database_mode() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "OPTIMIZE TABLE t", "--catalog", "./not-used"])
        .output()
        .unwrap();
    assert!(!result.status.success());
}
