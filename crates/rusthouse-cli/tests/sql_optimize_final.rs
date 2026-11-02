use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn sql(db: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

fn successful(db: &Path, statement: &str) -> Output {
    let output = sql(db, statement);
    assert!(output.status.success(), "{statement}: {output:?}");
    output
}

#[test]
fn optimize_final_preserves_rows() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-optimize_final_preserves_rows-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    successful(&db, "CREATE TABLE events (id UInt64)");
    successful(&db, "INSERT INTO events VALUES (1)");
    successful(&db, "INSERT INTO events VALUES (2)");
    successful(&db, "OPTIMIZE TABLE events FINAL");
    assert_eq!(successful(&db, "SELECT id FROM events ORDER BY id").stdout, b"id\\n1\\n2\\n");
    successful(&db, "OPTIMIZE TABLE events FINAL");
    assert_eq!(successful(&db, "SELECT count(*) FROM events").stdout, b"count\\n2\\n");
    assert!(!sql(&db, "OPTIMIZE TABLE events FINAL DEDUPLICATE").status.success());

    fs::remove_dir_all(db).unwrap();
}
