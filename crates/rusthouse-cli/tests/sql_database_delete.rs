use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-delete-sql-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(statement: &str, root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(root)
        .output()
        .unwrap()
}

#[test]
fn delete_where_preserves_unmatched_rows_and_supports_more_inserts() {
    let dir = root();
    assert!(
        sql(
            "CREATE TABLE events (id UInt64, name String, note Nullable(String))",
            &dir
        )
        .status
        .success()
    );
    assert!(
        sql(
            "INSERT INTO events VALUES (1, 'api', NULL), (2, 'db', 'ok'), (3, 'api', NULL)",
            &dir
        )
        .status
        .success()
    );
    assert!(
        sql("INSERT INTO events VALUES (4, 'worker', 'ok')", &dir)
            .status
            .success()
    );

    let removed = sql("DELETE FROM events WHERE name = 'api'", &dir);
    assert!(removed.status.success(), "{removed:?}");
    assert!(String::from_utf8_lossy(&removed.stdout).contains("Deleted 2 rows"));
    assert_eq!(
        sql("SELECT id, name FROM events ORDER BY id", &dir).stdout,
        b"id\tname\n2\tdb\n4\tworker\n"
    );

    let next = sql("INSERT INTO events VALUES (5, 'api', NULL)", &dir);
    assert!(next.status.success(), "{next:?}");
    let removed = sql("DELETE FROM events WHERE note IS NULL", &dir);
    assert!(removed.status.success(), "{removed:?}");
    assert_eq!(
        sql("SELECT count(*) FROM events", &dir).stdout,
        b"count\n2\n"
    );

    let removed = sql("DELETE FROM events", &dir);
    assert!(removed.status.success(), "{removed:?}");
    assert_eq!(
        sql("SELECT count(*) FROM events", &dir).stdout,
        b"count\n0\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_delete_does_not_change_stored_rows() {
    let dir = root();
    assert!(
        sql("CREATE TABLE t (id UInt8, name String)", &dir)
            .status
            .success()
    );
    assert!(
        sql("INSERT INTO t VALUES (5, 'hello')", &dir)
            .status
            .success()
    );
    for statement in [
        "DELETE FROM t WHERE missing = 1",
        "DELETE FROM t WHERE id = 'invalid'",
        "DELETE FROM t LIMIT 1",
        "DELETE FROM t; DROP TABLE t",
        "DELETE FROM absent",
    ] {
        let result = sql(statement, &dir);
        assert!(!result.status.success(), "{statement}: {result:?}");
        assert_eq!(sql("SELECT id FROM t", &dir).stdout, b"id\n5\n");
    }
    fs::remove_dir_all(dir).unwrap();
}
