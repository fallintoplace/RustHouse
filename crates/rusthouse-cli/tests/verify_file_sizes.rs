use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-file-size-cli-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    dir
}

fn manifest() -> Vec<u8> {
    let mut bytes = b"checksums format version: 3\n".to_vec();
    bytes.push(1); // one file
    bytes.push(6); // "id.bin"
    bytes.extend_from_slice(b"id.bin");
    bytes.push(3); // expected length
    bytes.extend_from_slice(&0u128.to_le_bytes());
    bytes.push(0); // not compressed
    bytes
}

fn run(dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("verify-file-sizes")
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn checks_manifest_sizes_and_reports_hash_limitation() {
    let dir = temp_dir();
    fs::write(dir.join("id.bin"), [1, 2, 3]).unwrap();
    fs::write(dir.join("checksums.txt"), manifest()).unwrap();
    let output = run(&dir);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Verified 1 file sizes, 3 total bytes"));
    assert!(stdout.contains("file hashes not verified"));

    fs::write(dir.join("id.bin"), [1, 2]).unwrap();
    let output = run(&dir);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("size differs"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_missing_and_extra_arguments() {
    for args in [
        vec!["verify-file-sizes"],
        vec!["verify-file-sizes", "a", "b"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
}
