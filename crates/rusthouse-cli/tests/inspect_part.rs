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
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rusthouse-storage/tests/fixtures/marks/wide.mrk2");
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
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rusthouse-storage/tests/fixtures/marks/fixed.mrk");
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

#[test]
fn inspect_blocks_rejects_directories() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-blocks")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected a regular file"));
}

#[test]
fn inspect_blocks_reports_offsets_and_sizes() {
    let path = std::env::temp_dir().join(format!("rusthouse-blocks-{}.bin", std::process::id()));
    let mut bytes = vec![0; 16];
    bytes.push(0x82);
    bytes.extend_from_slice(&13u32.to_le_bytes());
    bytes.extend_from_slice(&42u32.to_le_bytes());
    bytes.extend_from_slice(&[1, 2, 3, 4]);
    std::fs::write(&path, bytes).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-blocks")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("offset=0, method=0x82, compressed=13, decompressed=42"));
    assert!(stdout.contains("Blocks: 1 (headers only; checksums not verified)"));
}

#[test]
fn verify_blocks_rejects_checksum_mismatch() {
    let path = std::env::temp_dir().join(format!("rusthouse-verify-{}.bin", std::process::id()));
    let mut bytes = vec![0; 16];
    bytes.push(0x02);
    bytes.extend_from_slice(&14u32.to_le_bytes());
    bytes.extend_from_slice(&5u32.to_le_bytes());
    bytes.extend_from_slice(b"hello");
    std::fs::write(&path, bytes).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("verify-blocks")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"));
}

#[test]
fn verify_blocks_accepts_lz4_and_none_fixture() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rusthouse-storage/tests/fixtures/blocks/lz4-none.bin");
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("verify-blocks")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Verified 2 blocks, 85 decompressed bytes\n"
    );
}

#[test]
fn scan_u64_reads_values_without_loading_entire_column() {
    let fixture =
        include_bytes!("../../rusthouse-storage/tests/fixtures/blocks/lz4-none.bin");
    let path = std::env::temp_dir().join(format!("rusthouse-u64-{}.bin", std::process::id()));
    std::fs::write(&path, &fixture[..36]).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("scan-u64")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value = u64::from_le_bytes(*b"aaaaaaaa");
    assert!(stdout.contains("Rows: 10"));
    assert!(stdout.contains(&format!("Min: {value}")));
    assert!(stdout.contains(&format!("Max: {value}")));
}

#[test]
fn scan_u64_rejects_trailing_bytes() {
    let fixture =
        include_bytes!("../../rusthouse-storage/tests/fixtures/blocks/lz4-none.bin");
    let path = std::env::temp_dir().join(format!("rusthouse-u64-invalid-{}.bin", std::process::id()));
    std::fs::write(&path, fixture).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("scan-u64")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("incomplete 8-byte column value"));
}
