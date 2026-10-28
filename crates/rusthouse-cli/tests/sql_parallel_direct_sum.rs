use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn parallel_integer_sum_uses_filtered_numeric_parts() {
    let db = std::env::temp_dir().join(format!("rusthouse-parallel-sum-{}", std::process::id()));
    let create = sql("CREATE TABLE facts (value Int64, flag UInt8)", &db);
    assert!(create.status.success(), "{create:?}");
    for i in 0..32i64 {
        let statement = format!(
            "INSERT INTO facts VALUES ({i}, 1), ({}, 0), ({}, 1)",
            -i,
            i * 2
        );
        let result = sql(&statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        ("SELECT sum(value) FROM facts", "sum(value)\n992\n"),
        (
            "SELECT sum(value) FROM facts WHERE flag = 1",
            "sum(value)\n1488\n",
        ),
        (
            "SELECT sum(value) FROM facts WHERE flag = 0",
            "sum(value)\n-496\n",
        ),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            expected,
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
