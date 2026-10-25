use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

fn path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-output-{}-{}.txt",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn writes_sql_formats_to_a_new_file() {
    for (format, expected) in [
        ("tsv", "number\n0\n1\n2\n"),
        ("csv", "number\r\n0\r\n1\r\n2\r\n"),
        (
            "jsonl",
            "{\"number\":\"0\"}\n{\"number\":\"1\"}\n{\"number\":\"2\"}\n",
        ),
    ] {
        let output_path = path();
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args([
                "sql",
                "SELECT number FROM numbers(3)",
                "--format",
                format,
                "--output",
            ])
            .arg(&output_path)
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(result.stdout.is_empty());
        assert_eq!(fs::read_to_string(&output_path).unwrap(), expected);
        fs::remove_file(output_path).unwrap();
    }
}

#[test]
fn refuses_to_replace_existing_files_and_cleans_up_failed_queries() {
    let existing = path();
    fs::write(&existing, b"keep this").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT 1", "--output"])
        .arg(&existing)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(&existing).unwrap(), b"keep this");
    fs::remove_file(existing).unwrap();

    let failed = path();
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT missing FROM numbers(1)", "--output"])
        .arg(&failed)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!failed.exists());
}

#[test]
fn rejects_output_with_ast_or_duplicate_flags() {
    let output_path = path();
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT 1", "--show-ast", "--output"])
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output_path.exists());
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT 1", "--output"])
        .arg(&output_path)
        .args(["--output", "ignored.txt"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output_path.exists());
}
