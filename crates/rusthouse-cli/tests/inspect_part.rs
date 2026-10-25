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

#[test]
fn inspect_marks_prints_adaptive_offsets_and_rows() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../rusthouse-storage/tests/fixtures/marks/wide.mrk2");
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-marks")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Format: adaptive (.mrk2)"));
    assert!(stdout.contains("Marks: 2"));
    assert!(stdout.contains("compressed=90, decompressed=8, rows=10"));
}

#[test]
fn inspect_marks_prints_fixed_offsets() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../rusthouse-storage/tests/fixtures/marks/fixed.mrk");
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-marks")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Format: fixed (.mrk)"));
    assert!(stdout.contains("compressed=24, decompressed=64"));
    assert!(!stdout.contains("rows="));
}

#[test]
fn inspect_marks_rejects_compressed_files() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["inspect-marks", "marks.cmrk2"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("only uncompressed wide-part"));
}
