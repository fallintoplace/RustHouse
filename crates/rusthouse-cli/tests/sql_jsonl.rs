use std::process::Command;

#[test]
fn exports_numeric_rows_as_jsonl() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(3)", "--format", "jsonl"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "{\"number\":\"0\"}\n{\"number\":\"1\"}\n{\"number\":\"2\"}\n"
    );
}

#[test]
fn rejects_duplicate_column_names_in_jsonl() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT number, number FROM numbers(3)",
            "--format",
            "jsonl",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("unique projected"));
}
