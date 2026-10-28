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
fn typed_single_numeric_membership_handles_negative_values_and_nulls() {
    let db = std::env::temp_dir().join(format!("rusthouse-single-numeric-in-{}", std::process::id()));
    for statement in [
        "CREATE TABLE facts (id UInt64, amount Nullable(Int64))",
        "INSERT INTO facts VALUES (1, 10), (2, -10), (3, NULL), (4, 20)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        ("SELECT id FROM facts WHERE amount IN (10) ORDER BY id", "id\n1\n"),
        ("SELECT id FROM facts WHERE amount NOT IN (10) ORDER BY id", "id\n2\n4\n"),
        ("SELECT id FROM facts WHERE amount IN (-10) ORDER BY id", "id\n2\n"),
        ("SELECT id FROM facts WHERE amount NOT IN (-10) ORDER BY id", "id\n1\n4\n"),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
