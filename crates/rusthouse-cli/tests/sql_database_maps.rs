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
fn string_key_maps_round_trip_with_duplicate_keys_and_empty_rows() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-map-select-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    ok(
        &db,
        "CREATE TABLE events (id UInt64, attrs Map(String, UInt16), tags Map(String, String))",
    );
    ok(
        &db,
        r#"INSERT INTO events VALUES (2, '{}', '{}'), (1, '{"a":1,"a":2}', '{"k":"hello","b":"world"}')"#,
    );
    assert_eq!(
        ok(&db, "SELECT id, attrs FROM events ORDER BY id"),
        "id\tattrs\n1\t{\"a\":1,\"a\":2}\n2\t{}\n"
    );
    assert_eq!(
        ok(&db, "SELECT id, tags FROM events ORDER BY id"),
        "id\ttags\n1\t{\"k\":\"hello\",\"b\":\"world\"}\n2\t{}\n"
    );
    let invalid = sql(&db, r#"INSERT INTO events VALUES (3, '{"x":65536}', '{}')"#);
    assert!(!invalid.status.success());
    assert_eq!(ok(&db, "SELECT count(*) FROM events"), "count\n2\n");
    fs::remove_dir_all(db).unwrap();
}
