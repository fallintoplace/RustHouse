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
fn parallel_minmax_reduces_stored_integer_parts_and_filters() {
    let db = std::env::temp_dir().join(format!("rusthouse-parallel-minmax-{}", std::process::id()));
    let query = "CREATE TABLE facts (signed_value Int64, unsigned_value UInt64, active UInt8)";
    let output = sql(query, &db);
    assert!(output.status.success(), "{query}: {output:?}");
    for i in 0..32i64 {
        let query = format!(
            "INSERT INTO facts VALUES ({}, {}, {}), ({}, {}, 0)",
            i - 20,
            i,
            i % 2,
            i + 100,
            i + 100
        );
        let output = sql(&query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT min(signed_value) FROM facts",
            "min(signed_value)\n-20\n",
        ),
        (
            "SELECT max(unsigned_value) FROM facts",
            "max(unsigned_value)\n131\n",
        ),
        (
            "SELECT min(signed_value) FROM facts WHERE active = 1",
            "min(signed_value)\n-19\n",
        ),
        (
            "SELECT max(unsigned_value) FROM facts WHERE active = 1",
            "max(unsigned_value)\n31\n",
        ),
        (
            "SELECT min(signed_value) FROM facts WHERE active = 2",
            "min(signed_value)\nNULL\n",
        ),
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected,
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
