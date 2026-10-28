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
fn join_inner_empty_fast() {
    let database = std::env::temp_dir().join(format!("rusthouse-join_inner_empty_fast-{}", std::process::id()));
    for statement in ["CREATE TABLE empty (id UInt64)",
        "CREATE TABLE nonempty (id UInt64)",
        "INSERT INTO nonempty VALUES (42)"] {
        let result = sql(statement, &database);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    for query in [
        "SELECT e.id, n.id FROM empty e JOIN nonempty n ON e.id = n.id",
        "SELECT e.id, n.id FROM nonempty n JOIN empty e ON e.id = n.id",
        "SELECT e.id, n.id FROM empty e JOIN nonempty n ON e.id = n.id ORDER BY n.id",
    ] {
        let joined = sql(query, &database);
        assert!(joined.status.success(), "{query}: {joined:?}");
        assert_eq!(joined.stdout, b"e.id\tn.id\n");
    }

    fs::remove_dir_all(&database).unwrap();
}
