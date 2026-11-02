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

#[test]
fn truncate_if_exists_does_not_create_unknown_tables() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-truncate-if-exists-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let missing = sql(&db, "TRUNCATE TABLE IF EXISTS events");
    assert!(missing.status.success(), "{missing:?}");
    assert!(!db.join("events.schema").exists());
    assert!(!db.join("events.parts").exists());
    assert!(!sql(&db, "TRUNCATE TABLE events").status.success());

    assert!(sql(&db, "CREATE TABLE events (id UInt64)").status.success());
    assert!(sql(&db, "INSERT INTO events VALUES (9)").status.success());
    assert!(sql(&db, "TRUNCATE TABLE IF EXISTS events").status.success());
    let empty = sql(&db, "SELECT count(*) FROM events");
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"count\n0\n");

    fs::remove_file(db.join("events.schema")).unwrap();
    let orphan = sql(&db, "TRUNCATE TABLE IF EXISTS events");
    assert!(!orphan.status.success(), "{orphan:?}");
    assert!(String::from_utf8_lossy(&orphan.stderr).contains("orphaned data"));
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn truncate_parser_rejects_trailing_sql() {
    use rusthouse_query::{Ddl, parse_ddl};
    assert!(matches!(
        parse_ddl("TRUNCATE TABLE IF EXISTS missing").unwrap(),
        Some(Ddl::TruncateTableIfExists { name }) if name == "missing"
    ));
    for sql in [
        "TRUNCATE TABLE IF",
        "TRUNCATE TABLE IF NOT EXISTS events",
        "TRUNCATE TABLE IF EXISTS events; DROP TABLE events",
    ] {
        assert!(parse_ddl(sql).is_err(), "{sql}");
    }
}
