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
fn describe_short_forms_return_schema() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-describe_short_forms_return_schema-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    successful(&db, "CREATE TABLE events (id UInt64, label String)");
    let expected = b"name\\ttype\\nid\\tUInt64\\nlabel\\tString\\n";
    assert_eq!(successful(&db, "DESCRIBE events").stdout, expected);
    assert_eq!(successful(&db, "DESC events").stdout, expected);
    assert_eq!(successful(&db, "DESC TABLE events").stdout, expected);
    assert!(!sql(&db, "DESC events; DROP TABLE events").status.success());

    fs::remove_dir_all(db).unwrap();
}
