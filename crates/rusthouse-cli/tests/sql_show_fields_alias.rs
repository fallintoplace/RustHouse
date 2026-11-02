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
fn show_columns_and_fields_aliases_match() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-show-fields-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let created = sql(&db, "CREATE TABLE events (id UInt64, label String)");
    assert!(created.status.success(), "{created:?}");
    let expected = b"name\ttype\nid\tUInt64\nlabel\tString\n";
    for statement in [
        "SHOW COLUMNS IN events",
        "SHOW FIELDS FROM events",
        "SHOW FIELDS IN events",
    ] {
        let result = sql(&db, statement);
        assert!(result.status.success(), "{result:?}");
        assert_eq!(result.stdout, expected, "{statement}");
    }
    assert!(!sql(&db, "SHOW FIELDS events").status.success());
    fs::remove_dir_all(db).unwrap();
}
