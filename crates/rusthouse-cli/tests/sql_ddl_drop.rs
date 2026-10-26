use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn new_catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-drop-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn ddl(sql: &str, directory: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql, "--catalog"])
        .arg(directory)
        .output()
        .unwrap()
}

#[test]
fn drops_only_a_catalog_schema_and_supports_if_exists() {
    let dir = new_catalog();
    assert!(
        ddl("CREATE TABLE events (id UInt64)", &dir)
            .status
            .success()
    );
    fs::write(dir.join("real-part.bin"), b"untouched").unwrap();

    let dropped = ddl("DROP TABLE events", &dir);
    assert!(dropped.status.success(), "{dropped:?}");
    assert!(!dir.join("events.schema").exists());
    assert_eq!(fs::read(dir.join("real-part.bin")).unwrap(), b"untouched");

    let missing = ddl("DROP TABLE events", &dir);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("unknown catalog table"));

    let skipped = ddl("DROP TABLE IF EXISTS events", &dir);
    assert!(skipped.status.success(), "{skipped:?}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn does_not_delete_directories_named_like_tables() {
    let dir = new_catalog();
    fs::create_dir_all(dir.join("events.schema")).unwrap();
    let result = ddl("DROP TABLE events", &dir);
    assert!(!result.status.success());
    assert!(dir.join("events.schema").is_dir());
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn does_not_unlink_symlinks_as_catalog_tables() {
    let dir = new_catalog();
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join("target.txt");
    fs::write(&target, b"keep").unwrap();
    std::os::unix::fs::symlink(&target, dir.join("events.schema")).unwrap();

    let result = ddl("DROP TABLE events", &dir);
    assert!(!result.status.success());
    assert_eq!(fs::read(&target).unwrap(), b"keep");
    assert!(dir.join("events.schema").is_symlink());
    fs::remove_dir_all(dir).unwrap();
}
