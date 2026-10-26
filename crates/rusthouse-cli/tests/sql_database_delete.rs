use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-delete-{}-{}",
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
fn delete_filters_stored_rows_and_preserves_types() {
    let dir = database();
    assert!(
        sql(
            "CREATE TABLE events (id UInt64, note Nullable(String))",
            &dir
        )
        .status
        .success()
    );
    assert!(
        sql(
            "INSERT INTO events VALUES (1, 'one'), (2, NULL), (3, 'three')",
            &dir
        )
        .status
        .success()
    );
    let removed = sql("DELETE FROM events WHERE id >= 3 OR note IS NULL", &dir);
    assert!(removed.status.success(), "{removed:?}");
    assert_eq!(removed.stdout, b"Deleted 2 rows from events\n");
    let remaining = sql("SELECT id, note FROM events", &dir);
    assert_eq!(remaining.stdout, b"id\tnote\n1\tone\n");
    assert!(
        sql("INSERT INTO events VALUES (4, 'four')", &dir)
            .status
            .success()
    );
    let after = sql("SELECT count(*) FROM events", &dir);
    assert_eq!(after.stdout, b"count\n2\n");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_unbounded_and_invalid_predicates_without_writing() {
    let dir = database();
    assert!(
        sql("CREATE TABLE events (id UInt8, note String)", &dir)
            .status
            .success()
    );
    assert!(
        sql("INSERT INTO events VALUES (1, 'ok')", &dir)
            .status
            .success()
    );
    for statement in [
        "DELETE FROM events",
        "DELETE FROM events WHERE id = 'not a number'",
        "DELETE FROM events WHERE missing = 1",
        "DELETE FROM events WHERE id = 1; TRUNCATE TABLE events",
        "DELETE FROM events WHERE id = 1 ORDER BY id",
    ] {
        let deleted = sql(statement, &dir);
        assert!(!deleted.status.success(), "{statement}: {deleted:?}");
        assert_eq!(
            sql("SELECT count(*) FROM events", &dir).stdout,
            b"count\n1\n"
        );
    }
    let no_match = sql("DELETE FROM events WHERE id = 99", &dir);
    assert!(no_match.status.success(), "{no_match:?}");
    assert_eq!(no_match.stdout, b"Deleted 0 rows from events\n");
    fs::remove_dir_all(dir).unwrap();
}
