use std::path::Path;
use std::process::Command;

#[test]
fn verify_part_checks_streams_and_reports_limits() {
    let dir = std::env::temp_dir().join(format!("rusthouse-verify-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("count.txt"), "10\n").unwrap();
    std::fs::write(
        dir.join("columns.txt"),
        "columns format version: 1\n1 columns:\n`id` UInt64\n",
    )
    .unwrap();
    let blocks = include_bytes!("../../rusthouse-storage/tests/fixtures/blocks/lz4-none.bin");
    std::fs::write(dir.join("id.bin"), &blocks[..36]).unwrap();
    let mut marks = vec![0u8; 16];
    marks.extend_from_slice(&10u64.to_le_bytes());
    std::fs::write(dir.join("id.mrk2"), marks).unwrap();

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .arg("verify-part")
            .arg(&dir)
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Verified 1 streams, 1 marks, 1 blocks, 80 decompressed bytes"));
    assert!(stdout.contains("checksums.txt file hashes not verified"));

    let mut bytes = std::fs::read(dir.join("id.bin")).unwrap();
    bytes[0] ^= 1;
    std::fs::write(dir.join("id.bin"), bytes).unwrap();
    let output = run();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn verify_part_requires_one_directory_argument() {
    for args in [
        vec!["verify-part"],
        vec!["verify-part", "a", "b"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
}

#[test]
fn verify_part_rejects_a_nonexistent_directory() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("verify-part")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/missing-part"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("count.txt"));
}
