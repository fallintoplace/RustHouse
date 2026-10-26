use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(db: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn inserts_one_default_row_only_when_all_columns_are_nullable() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-default-values-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let create = sql(
        &db,
        "CREATE TABLE events (id Nullable(UInt64), name Nullable(String))",
    );
    assert!(create.status.success(), "{create:?}");

    let insert = sql(&db, "INSERT INTO events DEFAULT VALUES");
    assert!(insert.status.success(), "{insert:?}");
    assert_eq!(
        sql(&db, "SELECT id, name FROM events").stdout,
        b"id\tname\n\\N\t\\N\n"
    );

    let bad = sql(&db, "CREATE TABLE required (id UInt64)");
    assert!(bad.status.success(), "{bad:?}");
    let failed = sql(&db, "INSERT INTO required DEFAULT VALUES");
    assert!(!failed.status.success(), "{failed:?}");
    assert!(String::from_utf8_lossy(&failed.stderr).contains("missing or NULL"));
    assert_eq!(
        sql(&db, "SELECT count(*) FROM required").stdout,
        b"count\n0\n"
    );

    fs::remove_dir_all(db).unwrap();
}
