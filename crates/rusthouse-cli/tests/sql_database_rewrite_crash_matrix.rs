//! Regression coverage for copy-on-write UPDATE and OPTIMIZE publication.
//! These deliberately terminate only disposable subprocesses in debug builds.
#![cfg(debug_assertions)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-rewrite-crash-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    dir
}

fn sql(dir: &Path, statement: &str, crash: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rusthouse"));
    cmd.args(["sql", statement, "--database"]).arg(dir);
    cmd.env_remove("RUSTHOUSE_DEBUG_CRASH_AT");
    if let Some(point) = crash {
        cmd.env("RUSTHOUSE_DEBUG_CRASH_AT", point);
    }
    cmd.output().unwrap()
}

fn ok(dir: &Path, statement: &str) -> String {
    let result = sql(dir, statement, None);
    assert!(
        result.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

fn doctor(dir: &Path) {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["doctor", "--database"])
        .arg(dir)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
}

#[test]
fn update_is_visible_either_entirely_before_or_after_pointer_switch() {
    for (point, visible_name) in [
        ("snapshot-after-staging", "before"),
        ("snapshot-after-generation", "before"),
        ("snapshot-before-pointer", "before"),
        ("snapshot-after-pointer", "after"),
    ] {
        let dir = database();
        ok(&dir, "CREATE TABLE events (id UInt64, name String)");
        ok(&dir, "INSERT INTO events VALUES (1, 'before'), (2, 'keep')");
        let interrupted = sql(
            &dir,
            "UPDATE events SET name = 'after' WHERE id = 1",
            Some(point),
        );
        assert_eq!(interrupted.status.code(), Some(86), "{point}: {interrupted:?}");
        assert_eq!(
            ok(&dir, "SELECT id, name FROM events ORDER BY id"),
            format!("id\tname\n1\t{visible_name}\n2\tkeep\n"),
            "{point}"
        );
        doctor(&dir);
        ok(&dir, "INSERT INTO events VALUES (3, 'next')");
        assert_eq!(ok(&dir, "SELECT count(*) FROM events"), "count\n3\n");
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn optimize_preserves_all_rows_across_each_publication_boundary() {
    for point in [
        "snapshot-after-staging",
        "snapshot-after-generation",
        "snapshot-before-pointer",
        "snapshot-after-pointer",
    ] {
        let dir = database();
        ok(&dir, "CREATE TABLE events (id UInt64)");
        ok(&dir, "INSERT INTO events VALUES (1), (2)");
        ok(&dir, "INSERT INTO events VALUES (3), (4)");
        let interrupted = sql(&dir, "OPTIMIZE TABLE events", Some(point));
        assert_eq!(interrupted.status.code(), Some(86), "{point}: {interrupted:?}");
        assert_eq!(
            ok(&dir, "SELECT id FROM events ORDER BY id"),
            "id\n1\n2\n3\n4\n",
            "{point}"
        );
        doctor(&dir);
        ok(&dir, "INSERT INTO events VALUES (5)");
        assert_eq!(ok(&dir, "SELECT count(*) FROM events"), "count\n5\n");
        fs::remove_dir_all(dir).unwrap();
    }
}
