use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-update-sql-{}-{}",
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

#[test]
fn update_where_replaces_matching_rows_only_and_persists() {
    let dir = root();
    assert!(
        sql(
            "CREATE TABLE t (id UInt64, state String, note Nullable(String))",
            &dir
        )
        .status
        .success()
    );
    assert!(
        sql(
            "INSERT INTO t VALUES (1, 'open', NULL), (2, 'open', 'old'), (3, 'closed', 'keep')",
            &dir
        )
        .status
        .success()
    );

    let update = sql(
        "UPDATE t SET state = 'done', note = 'changed' WHERE id = 2",
        &dir,
    );
    assert!(update.status.success(), "{update:?}");
    assert!(String::from_utf8_lossy(&update.stdout).contains("Updated 1 rows"));
    assert_eq!(
        sql("SELECT id, state FROM t ORDER BY id", &dir).stdout,
        b"id\tstate\n1\topen\n2\tdone\n3\tclosed\n"
    );

    let nullable = sql("UPDATE t SET note = NULL WHERE state = 'closed'", &dir);
    assert!(nullable.status.success(), "{nullable:?}");
    assert_eq!(
        sql("SELECT count(note) FROM t", &dir).stdout,
        b"count(note)\n1\n"
    );

    let more = sql("INSERT INTO t VALUES (4, 'open', NULL)", &dir);
    assert!(more.status.success(), "{more:?}");
    let all = sql("UPDATE t SET state = 'archived'", &dir);
    assert!(all.status.success(), "{all:?}");
    assert_eq!(
        sql("SELECT count(*) FROM t WHERE state = 'archived'", &dir).stdout,
        b"count\n4\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_update_keeps_old_data_visible() {
    let dir = root();
    assert!(
        sql("CREATE TABLE t (id UInt8, name String)", &dir)
            .status
            .success()
    );
    assert!(
        sql("INSERT INTO t VALUES (5, 'before')", &dir)
            .status
            .success()
    );
    for statement in [
        "UPDATE t SET id = 999",
        "UPDATE t SET name = NULL",
        "UPDATE t SET missing = 'x'",
        "UPDATE t SET name = 'x' WHERE invalid = 1",
        "UPDATE t SET id = 1; DROP TABLE t",
        "UPDATE t SET id = 6 WHERE id = 'bad'",
    ] {
        let result = sql(statement, &dir);
        assert!(!result.status.success(), "{statement}: {result:?}");
        assert_eq!(
            sql("SELECT id, name FROM t", &dir).stdout,
            b"id\tname\n5\tbefore\n"
        );
    }
    fs::remove_dir_all(dir).unwrap();
}
