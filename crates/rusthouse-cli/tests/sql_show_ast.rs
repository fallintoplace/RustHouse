use std::process::Command;

#[test]
fn shows_parsed_sql_without_touching_part_files() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "--show-ast", "SELECT id FROM part WHERE id >= 3 LIMIT 2"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let result = String::from_utf8(output.stdout).unwrap();
    assert!(result.contains("Table("));
    assert!(result.contains("Comparison"));
    assert!(result.contains("limit: Some("));
    assert!(result.contains("2"));
}

#[test]
fn rejects_invalid_sql_and_duplicate_ast_flags() {
    let bad_query = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "--show-ast", "DROP TABLE part"])
        .output()
        .unwrap();
    assert!(!bad_query.status.success());

    let duplicate = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT 1", "--show-ast", "--show-ast"])
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate"));
}
