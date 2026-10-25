use std::process::Command;

#[test]
fn emits_only_numeric_csv_rows() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT number FROM numbers(4) WHERE number >= 2",
            "--format",
            "csv-no-header",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"2\r\n3\r\n");
}

#[test]
fn csv_without_headers_handles_empty_rows() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT number FROM numbers(0)",
            "--format",
            "csv-no-header",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stdout.is_empty());
}

#[test]
fn old_csv_format_keeps_its_header_for_compatibility() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(2)", "--format", "csv"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"number\r\n0\r\n1\r\n");
}
