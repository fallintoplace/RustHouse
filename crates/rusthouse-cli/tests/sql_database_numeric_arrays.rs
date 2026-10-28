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

fn ok(db: &Path, statement: &str) -> String {
    let output = sql(db, statement);
    assert!(
        output.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn numeric_arrays_round_trip_through_named_database_tables() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-array-sql-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    ok(&db, "CREATE TABLE events (id UInt64, numbers Array(UInt16))");
    ok(
        &db,
        "INSERT INTO events VALUES (2, '[]'), (1, '[2, 65535]'), (3, '[0, 7]')",
    );
    assert_eq!(
        ok(&db, "SELECT id, numbers FROM events ORDER BY id"),
        "id\tnumbers\n1\t[2,65535]\n2\t[]\n3\t[0,7]\n"
    );
    assert_eq!(ok(&db, "SELECT count(*) FROM events"), "count\n3\n");
    let error = sql(&db, "INSERT INTO events VALUES (4, '[65536]')");
    assert!(!error.status.success());
    assert_eq!(ok(&db, "SELECT count(*) FROM events"), "count\n3\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn all_empty_array_granules_are_readable() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-empty-array-sql-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    ok(&db, "CREATE TABLE items (values Array(Int8))");
    ok(&db, "INSERT INTO items VALUES ('[]'), ('[]')");
    assert_eq!(ok(&db, "SELECT values FROM items"), "values\n[]\n[]\n");
    fs::remove_dir_all(db).unwrap();
}
