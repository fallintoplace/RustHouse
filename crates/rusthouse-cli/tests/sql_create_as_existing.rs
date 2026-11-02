use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn sql(database: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

fn successful(database: &Path, statement: &str) -> Output {
    let output = sql(database, statement);
    assert!(output.status.success(), "{statement}: {output:?}");
    output
}

#[test]
fn create_as_clones_schema_without_copying_data() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-create-as-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    successful(&db, "CREATE TABLE original (id UInt64, message Nullable(String))");
    successful(&db, "INSERT INTO original VALUES (1, 'source')");
    successful(&db, "CREATE TABLE cloned AS original");
    assert_eq!(
        successful(&db, "DESCRIBE TABLE cloned").stdout,
        b"name\ttype\nid\tUInt64\nmessage\tNullable(String)\n"
    );
    assert_eq!(
        successful(&db, "SELECT count(*) FROM cloned").stdout,
        b"count\n0\n"
    );
    successful(&db, "INSERT INTO cloned VALUES (2, NULL)");
    assert_eq!(
        successful(&db, "SELECT count(*) FROM original").stdout,
        b"count\n1\n"
    );
    assert!(!sql(&db, "CREATE TABLE bad AS missing").status.success());
    fs::remove_dir_all(db).unwrap();
}
