use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn join_chain_empty_fast() {
    let database = std::env::temp_dir().join(format!("rusthouse-join_chain_empty_fast-{}", std::process::id()));
    for statement in ["CREATE TABLE a (id UInt64)",
        "CREATE TABLE b (id UInt64)",
        "CREATE TABLE c (id UInt64)",
        "INSERT INTO a VALUES (1)",
        "INSERT INTO b VALUES (2)",
        "INSERT INTO c VALUES (3)"] {
        let result = sql(statement, &database);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    let empty = sql(
        "SELECT a.id, b.id, c.id FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id",
        &database,
    );
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"a.id\tb.id\tc.id\n");
    let full = sql(
        "SELECT a.id, b.id, c.id FROM a JOIN b ON a.id = b.id FULL JOIN c ON b.id = c.id",
        &database,
    );
    assert!(full.status.success(), "{full:?}");
    assert_eq!(full.stdout, b"a.id\tb.id\tc.id\n\\N\t\\N\t3\n");

    fs::remove_dir_all(&database).unwrap();
}
