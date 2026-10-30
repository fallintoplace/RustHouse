use std::fs;
use std::process::Command;

#[test]
fn chained_inner_join_streams_facts_larger_than_the_materialization_limit() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-chain-stream-large-fact-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimensions (id UInt64)",
        "CREATE TABLE regions (id UInt64)",
        "INSERT INTO facts SELECT number FROM numbers(40000)",
        "INSERT INTO dimensions VALUES (39998), (39999)",
        "INSERT INTO regions VALUES (39998), (39999)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (statement, expected) in [
        (
            "SELECT f.id FROM facts f JOIN dimensions d ON f.id = d.id JOIN regions r ON d.id = r.id LIMIT 2",
            b"f.id\n39998\n39999\n".as_slice(),
        ),
        (
            "SELECT f.id FROM facts f JOIN dimensions d ON f.id = d.id JOIN regions r ON d.id = r.id LIMIT 1 OFFSET 1",
            b"f.id\n39999\n".as_slice(),
        ),
        (
            "SELECT f.id FROM facts f ANY INNER JOIN dimensions d ON f.id = d.id ANY INNER JOIN regions r ON d.id = r.id LIMIT 2",
            b"f.id\n39998\n39999\n".as_slice(),
        ),
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
        assert_eq!(output.stdout, expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
