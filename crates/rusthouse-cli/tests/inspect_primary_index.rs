use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

fn path(extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-index-cli-{}-{}.{}",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed),
        extension
    ))
}

#[test]
fn previews_fixed_width_primary_index_fields_as_bytes() {
    let index_path = path("idx");
    let mut bytes = Vec::new();
    for (first, second) in [(7u32, 9u16), (u32::MAX, 10u16)] {
        bytes.extend_from_slice(&first.to_le_bytes());
        bytes.extend_from_slice(&second.to_le_bytes());
    }
    fs::write(&index_path, bytes).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-primary-index")
        .arg(&index_path)
        .args(["4", "2"])
        .output()
        .unwrap();
    fs::remove_file(index_path).unwrap();
    assert!(result.status.success(), "{result:?}");
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("Marks: 2"));
    assert!(output.contains("Fields: 2"));
    assert!(output.contains("Mark 0: 07000000 0900"));
    assert!(output.contains("Mark 1: ffffffff 0a00"));
}

#[test]
fn rejects_invalid_and_missing_field_widths() {
    for args in [
        vec!["inspect-primary-index", "missing.idx"],
        vec!["inspect-primary-index", "missing.idx", "0"],
        vec!["inspect-primary-index", "missing.idx", "-1"],
        vec!["inspect-primary-index", "missing.idx", "not-a-width"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{args:?}");
    }
}

#[test]
fn previews_only_a_bounded_number_of_marks() {
    let index_path = path("idx");
    fs::write(
        &index_path,
        (0u32..20).flat_map(u32::to_le_bytes).collect::<Vec<_>>(),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-primary-index")
        .arg(&index_path)
        .arg("4")
        .output()
        .unwrap();
    fs::remove_file(index_path).unwrap();
    assert!(result.status.success(), "{result:?}");
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("Marks: 20"));
    assert!(output.contains("Mark 15:"));
    assert!(!output.contains("Mark 16:"));
    assert!(output.contains("... 4 more marks"));
}
