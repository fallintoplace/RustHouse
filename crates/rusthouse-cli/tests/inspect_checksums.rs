use std::process::Command;

#[test]
fn inspect_checksums_reports_binary_entries_without_claiming_verification() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-checksums-cli-{}.txt",
        std::process::id()
    ));
    let mut data = b"checksums format version: 3\n".to_vec();
    data.push(1);
    data.push(6);
    data.extend_from_slice(b"id.bin");
    data.push(80);
    data.extend_from_slice(&42u128.to_le_bytes());
    data.push(0);
    std::fs::write(&path, data).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-checksums")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(&path).unwrap();

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Format: 3"));
    assert!(stdout.contains("Files: 1"));
    assert!(stdout.contains("\"id.bin\": 80 bytes"));
    assert!(stdout.contains("not verified"));
}

#[test]
fn inspect_checksums_rejects_missing_file_and_extra_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["inspect-checksums", "missing-checksums.txt"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));

    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["inspect-checksums", "a", "b"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}
