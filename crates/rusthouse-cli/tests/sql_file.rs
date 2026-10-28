use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

fn sql_file(contents: &[u8]) -> std::path::PathBuf {
    let file = std::env::temp_dir().join(format!(
        "rusthouse-sql-file-{}-{}.sql",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&file, contents).unwrap();
    file
}

#[test]
fn executes_sql_from_a_file() {
    let file = sql_file(b"SELECT number FROM numbers(10) WHERE number >= 7 LIMIT 2");
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "number\n7\n8\n");
    fs::remove_file(file).unwrap();
}

#[test]
fn rejects_oversized_and_non_utf8_sql_files() {
    let too_large = sql_file(&vec![b' '; 65537]);
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "--file"])
        .arg(&too_large)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("65536-byte"));
    fs::remove_file(too_large).unwrap();

    let invalid = sql_file(&[0xff]);
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "--file"])
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("valid UTF-8"));
    fs::remove_file(invalid).unwrap();
}

#[test]
fn rejects_conflicting_sql_sources() {
    let file = sql_file(b"SELECT 1");
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT 2", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("not both"));
    fs::remove_file(file).unwrap();
}
