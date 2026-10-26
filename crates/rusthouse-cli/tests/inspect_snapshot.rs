use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-snapshot-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

fn create_part(root: &Path, name: &str, rows: u64) {
    let directory = root.join(name);
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("count.txt"), format!("{rows}\n")).unwrap();
    fs::write(
        directory.join("columns.txt"),
        "columns format version: 1\n1 columns:\n`id` UInt64\n",
    )
    .unwrap();
}

fn inspect(root: &Path, extra: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rusthouse"));
    cmd.arg("inspect-snapshot").arg(root).args(extra);
    cmd.output().unwrap()
}

#[test]
fn summarizes_multiple_active_parts_as_text_and_json() {
    let path = root();
    create_part(&path, "all_1_1_0", 3);
    create_part(&path, "all_2_2_0", 5);
    let text = inspect(&path, &[]);
    assert!(text.status.success(), "{text:?}");
    assert_eq!(
        String::from_utf8(text.stdout).unwrap(),
        "Parts: 2\nRows: 8\nColumns: 1\n  \"id\": UInt64\n"
    );
    let json = inspect(&path, &["--json"]);
    assert!(json.status.success(), "{json:?}");
    let stdout = String::from_utf8(json.stdout).unwrap();
    assert!(stdout.contains("\"parts\":2,\"rows\":8"));
    assert!(stdout.contains("\"columns\":[{\"name\":\"id\",\"type\":\"UInt64\"}]"));
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn refuses_overlapping_ranges_and_schema_mismatches() {
    let path = root();
    create_part(&path, "all_1_2_0", 3);
    create_part(&path, "all_2_3_0", 5);
    let overlap = inspect(&path, &[]);
    assert!(!overlap.status.success());
    assert!(String::from_utf8_lossy(&overlap.stderr).contains("overlapping"));
    fs::remove_dir_all(&path).unwrap();

    let path = root();
    create_part(&path, "all_1_1_0", 3);
    create_part(&path, "all_2_2_0", 5);
    fs::write(
        path.join("all_2_2_0").join("columns.txt"),
        "columns format version: 1\n1 columns:\n`different` UInt64\n",
    )
    .unwrap();
    let mismatch = inspect(&path, &[]);
    assert!(!mismatch.status.success());
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("schema mismatch"));
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn rejects_unrecognized_flags_without_scanning_files() {
    let path = root();
    let invalid = inspect(&path, &["--broken"]);
    assert_eq!(invalid.status.code(), Some(2));
    fs::remove_dir_all(path).unwrap();
}
