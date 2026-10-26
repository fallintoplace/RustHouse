use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-truncate-{}-{}",
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
fn truncation_preserves_schema_and_allows_new_inserts() {
    let dir = database();
    let create = sql("CREATE TABLE events (id UInt64, note Nullable(String))", &dir);
    assert!(create.status.success(), "{create:?}");
    assert!(sql("INSERT INTO events VALUES (1, 'first'), (2, NULL)", &dir).status.success());
    assert_eq!(sql("SELECT count(*) FROM events", &dir).stdout, b"count\n2\n");

    let truncate = sql("TRUNCATE TABLE events", &dir);
    assert!(truncate.status.success(), "{truncate:?}");
    assert_eq!(sql("SELECT count(*) FROM events", &dir).stdout, b"count\n0\n");
    assert_eq!(
        fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\nnote\tNullable(String)\n"
    );
    assert_eq!(fs::read_dir(dir.join("events.parts")).unwrap().count(), 1);
    assert!(sql("INSERT INTO events VALUES (3, 'new')", &dir).status.success());
    assert_eq!(sql("SELECT id FROM events", &dir).stdout, b"id\n3\n");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_truncate_does_not_modify_data() {
    let dir = database();
    assert!(sql("CREATE TABLE t (id UInt64)", &dir).status.success());
    assert!(sql("INSERT INTO t VALUES (7)", &dir).status.success());
    for statement in [
        "TRUNCATE TABLE missing",
        "TRUNCATE TABLE t; DROP TABLE t",
        "TRUNCATE TABLE t INVALID",
    ] {
        let result = sql(statement, &dir);
        assert!(!result.status.success(), "{statement}: {result:?}");
        assert_eq!(sql("SELECT id FROM t", &dir).stdout, b"id\n7\n");
    }
    fs::remove_dir_all(dir).unwrap();
}
