use std::path::Path;
use std::process::Command;

#[test]
fn reads_uint64_values_from_checked_clickhouse_blocks() {
    let bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rusthouse-storage/tests/fixtures/blocks/lz4-none.bin");
    let marks = std::env::temp_dir().join(format!(
        "rusthouse-u64-{}-{}.mrk2",
        std::process::id(),
        "cli"
    ));
    let mut mark_bytes = Vec::new();
    mark_bytes.extend_from_slice(&0u64.to_le_bytes());
    mark_bytes.extend_from_slice(&8u64.to_le_bytes());
    mark_bytes.extend_from_slice(&2u64.to_le_bytes());
    std::fs::write(&marks, mark_bytes).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("read-u64")
        .arg(&bin)
        .arg(&marks)
        .arg("0")
        .output()
        .unwrap();
    let invalid = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("read-u64")
        .arg(&bin)
        .arg(&marks)
        .arg("1")
        .output()
        .unwrap();
    std::fs::remove_file(marks).unwrap();

    let value = u64::from_le_bytes(*b"aaaaaaaa");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("Rows: 2\n{value}\n{value}\n")
    );
    assert_eq!(invalid.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("outside 1 marks"));
}

#[test]
fn rejects_non_numeric_mark_index() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["read-u64", "x.bin", "x.mrk2", "-1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
}
