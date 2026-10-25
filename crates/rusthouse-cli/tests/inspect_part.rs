use std::path::Path;
use std::process::Command;

#[test]
fn inspect_part_prints_rows_and_columns() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../rusthouse-storage/tests/fixtures/all_1_1_0");
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-part")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Rows: 42"));
    assert!(stdout.contains("\"city\": LowCardinality(String)"));
}

#[test]
fn wrong_arguments_fail_with_usage() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["inspect-part"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr).unwrap().contains("Usage:"));
}

#[test]
fn missing_part_reports_file_path() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/missing");
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-part")
        .arg(path)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("count.txt"));
}
