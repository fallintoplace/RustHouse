//! The optional offline doctor audit verifies generations retained for recovery.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-doctor-generations-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(root: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(root)
        .output()
        .unwrap()
}

fn doctor(root: &Path, flags: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("doctor")
        .arg("--database")
        .arg(root)
        .args(flags)
        .output()
        .unwrap()
}

fn succeeds(result: Output) -> Output {
    assert!(result.status.success(), "{result:?}");
    result
}

#[test]
fn opt_in_audit_detects_broken_retained_generation() {
    let root = database();
    succeeds(sql(&root, "CREATE TABLE events (id UInt64)"));
    succeeds(sql(&root, "INSERT INTO events VALUES (1), (2)"));
    succeeds(sql(&root, "DELETE FROM events WHERE id = 1"));
    succeeds(sql(&root, "UPDATE events SET id = 3 WHERE id = 2"));

    assert_eq!(
        succeeds(sql(&root, "SELECT id FROM events")).stdout,
        b"id\n3\n"
    );
    let healthy = succeeds(doctor(&root, &["--all-generations", "--full-hashes"]));
    assert!(
        String::from_utf8_lossy(&healthy.stdout).contains("OK events@g1"),
        "{healthy:?}"
    );
    assert!(
        String::from_utf8_lossy(&healthy.stdout).contains("OK events: 1 rows"),
        "{healthy:?}"
    );

    let old_stream = root.join("events.parts/.rusthouse-versions/g1/all_1_1_0/id.bin");
    fs::remove_file(&old_stream).unwrap();
    let normal = succeeds(doctor(&root, &[]));
    assert!(
        String::from_utf8_lossy(&normal.stdout).contains("0 error(s)"),
        "{normal:?}"
    );
    let audit = doctor(&root, &["--all-generations"]);
    assert!(!audit.status.success(), "{audit:?}");
    assert!(
        String::from_utf8_lossy(&audit.stdout).contains("ERROR events@g1"),
        "{audit:?}"
    );
    assert!(!old_stream.exists(), "doctor must not repair user data");
    assert_eq!(
        succeeds(sql(&root, "SELECT id FROM events")).stdout,
        b"id\n3\n"
    );
    fs::remove_dir_all(root).unwrap();
}
