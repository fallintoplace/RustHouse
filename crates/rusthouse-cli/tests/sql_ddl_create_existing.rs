use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-existing-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn create_if_not_exists(dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "CREATE TABLE IF NOT EXISTS events (id UInt64)", "--catalog"])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn create_if_not_exists_accepts_valid_schema_but_rejects_corrupt_files() {
    let dir = catalog();
    let initial = create_if_not_exists(&dir);
    assert!(initial.status.success(), "{initial:?}");
    assert!(create_if_not_exists(&dir).status.success());

    let schema = dir.join("events.schema");
    fs::write(&schema, "broken schema\n").unwrap();
    let invalid = create_if_not_exists(&dir);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid schema version"));
    assert_eq!(fs::read_to_string(&schema).unwrap(), "broken schema\n");
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn create_if_not_exists_rejects_schema_symlinks() {
    let dir = catalog();
    fs::create_dir_all(&dir).unwrap();
    let outside = dir.join("outside.txt");
    fs::write(&outside, b"leave unchanged").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("events.schema")).unwrap();

    let result = create_if_not_exists(&dir);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("not a regular schema file"));
    assert_eq!(fs::read(&outside).unwrap(), b"leave unchanged");
    fs::remove_dir_all(dir).unwrap();
}
