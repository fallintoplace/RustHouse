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
fn single_string_in_uses_comparison_semantics_for_nullable_rows() {
    let db = std::env::temp_dir().join(format!("rusthouse-single-text-in-{}", std::process::id()));
    for statement in [
        "CREATE TABLE events (id UInt64, tag Nullable(String))",
        "INSERT INTO events VALUES (1, 'red'), (2, 'blue'), (3, NULL), (4, 'red')",
    ] {
        let output = sql(statement, &db);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (statement, expected) in [
        ("SELECT id FROM events WHERE tag IN ('red') ORDER BY id", "id\n1\n4\n"),
        ("SELECT id FROM events WHERE tag NOT IN ('red') ORDER BY id", "id\n2\n"),
        ("SELECT id FROM events WHERE tag IN ('missing') ORDER BY id", "id\n"),
    ] {
        let output = sql(statement, &db);
        assert!(output.status.success(), "{statement}: {output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
