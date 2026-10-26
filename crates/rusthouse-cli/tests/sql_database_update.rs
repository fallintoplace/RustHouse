use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-update-{}-{}",
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

fn ok(statement: &str, dir: &Path) -> Output {
    let result = sql(statement, dir);
    assert!(result.status.success(), "{statement}: {result:?}");
    result
}

#[test]
fn updates_matching_rows_without_touching_other_rows() {
    let dir = database();
    ok("CREATE TABLE events (id UInt8, name String, note Nullable(String))", &dir);
    ok("INSERT INTO events VALUES (1, 'Ada', NULL), (2, 'Bob', 'old')", &dir);
    let changed = ok(
        "UPDATE events SET name = 'Grace', note = NULL WHERE id = 2",
        &dir,
    );
    assert_eq!(changed.stdout, b"Updated 1 rows in events\n");
    let result = ok("SELECT id, name, note FROM events ORDER BY id", &dir);
    assert_eq!(result.stdout, b"id\tname\tnote\n1\tAda\t\\N\n2\tGrace\t\\N\n");
    ok("INSERT INTO events VALUES (3, 'Lin', 'fresh')", &dir);
    assert_eq!(ok("SELECT count(*) FROM events", &dir).stdout, b"count\n3\n");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_update_is_atomic_for_existing_table() {
    let dir = database();
    ok("CREATE TABLE events (id UInt8, name String)", &dir);
    ok("INSERT INTO events VALUES (1, 'original')", &dir);
    for statement in [
        "UPDATE events SET name = 'oops'",
        "UPDATE events SET missing = 'oops' WHERE id = 1",
        "UPDATE events SET name = NULL WHERE id = 1",
        "UPDATE events SET id = 256 WHERE id = 1",
        "UPDATE events SET id = 2, id = 3 WHERE id = 1",
    ] {
        let result = sql(statement, &dir);
        assert!(!result.status.success(), "{statement}: {result:?}");
        let current = ok("SELECT id, name FROM events", &dir);
        assert_eq!(current.stdout, b"id\tname\n1\toriginal\n");
    }
    let zero = ok("UPDATE events SET name = 'new' WHERE id = 50", &dir);
    assert_eq!(zero.stdout, b"Updated 0 rows in events\n");
    fs::remove_dir_all(dir).unwrap();
}
