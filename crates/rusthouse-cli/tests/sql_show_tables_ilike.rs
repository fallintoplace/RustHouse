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
fn show_tables_ilike_matches_ascii_case_without_changing_names() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-show-tables-ilike-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    for sql_text in [
        "CREATE TABLE Events (id UInt64)",
        "CREATE TABLE extras (id UInt64)",
        "CREATE TABLE logs (id UInt64)",
    ] {
        let result = sql(&db, sql_text);
        assert!(result.status.success(), "{result:?}");
    }
    assert_eq!(sql(&db, "SHOW TABLES ILIKE 'EV%'").stdout, b"name\nEvents\n");
    assert_eq!(sql(&db, "SHOW TABLES LIKE 'EV%'").stdout, b"name\n");
    assert_eq!(
        sql(&db, "SHOW TABLES NOT ILIKE 'EV%'").stdout,
        b"name\nextras\nlogs\n"
    );
    fs::remove_dir_all(db).unwrap();
}
