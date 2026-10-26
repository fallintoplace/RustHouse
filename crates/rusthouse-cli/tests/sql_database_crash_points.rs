//! Process-exit crash cases for local database publication.
#![cfg(debug_assertions)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DB: AtomicUsize = AtomicUsize::new(0);
const CRASH_EXIT: i32 = 86;

fn database() -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "rusthouse-process-crash-{}-{}",
        std::process::id(),
        NEXT_DB.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    directory
}

fn run(directory: &Path, query: &str, crash_at: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusthouse"));
    command.args(["sql", query, "--database"]).arg(directory);
    if let Some(point) = crash_at {
        command.env("RUSTHOUSE_DEBUG_CRASH_AT", point);
    } else {
        command.env_remove("RUSTHOUSE_DEBUG_CRASH_AT");
    }
    command.output().unwrap()
}

fn success(directory: &Path, query: &str) -> Output {
    let result = run(directory, query, None);
    assert!(
        result.status.success(),
        "{query}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

#[test]
fn snapshot_publication_survives_process_exit_at_each_boundary() {
    for (point, expected) in [
        ("snapshot-after-staging", "id\n1\n2\n3\n"),
        ("snapshot-after-generation", "id\n1\n2\n3\n"),
        ("snapshot-before-pointer", "id\n1\n2\n3\n"),
        ("snapshot-after-pointer", "id\n2\n3\n"),
    ] {
        let dir = database();
        success(&dir, "CREATE TABLE events (id UInt64)");
        success(&dir, "INSERT INTO events VALUES (1), (2), (3)");

        let crashed = run(&dir, "DELETE FROM events WHERE id = 1", Some(point));
        assert_eq!(
            crashed.status.code(),
            Some(CRASH_EXIT),
            "{point}: {crashed:?}"
        );

        // Restarting the CLI must see exactly one complete generation.
        assert_eq!(
            success(&dir, "SELECT id FROM events ORDER BY id").stdout,
            expected.as_bytes(),
            "{point}"
        );
        // The exited writer must not block future commits.
        success(&dir, "INSERT INTO events VALUES (4)");
        assert_eq!(
            success(&dir, "SELECT id FROM events ORDER BY id").stdout,
            format!("{expected}4\n").as_bytes(),
            "{point}"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn inserted_parts_are_never_partially_exposed_after_process_exit() {
    for (point, expected) in [
        ("insert-after-staging", "id\n1\n"),
        ("insert-after-rename", "id\n1\n2\n"),
    ] {
        let dir = database();
        success(&dir, "CREATE TABLE events (id UInt64)");
        success(&dir, "INSERT INTO events VALUES (1)");
        let crashed = run(&dir, "INSERT INTO events VALUES (2)", Some(point));
        assert_eq!(
            crashed.status.code(),
            Some(CRASH_EXIT),
            "{point}: {crashed:?}"
        );
        assert_eq!(
            success(&dir, "SELECT id FROM events ORDER BY id").stdout,
            expected.as_bytes(),
            "{point}"
        );
        success(&dir, "INSERT INTO events VALUES (3)");
        let expected_after = if point == "insert-after-staging" {
            "id\n1\n3\n"
        } else {
            "id\n1\n2\n3\n"
        };
        assert_eq!(
            success(&dir, "SELECT id FROM events ORDER BY id").stdout,
            expected_after.as_bytes(),
            "{point}"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
