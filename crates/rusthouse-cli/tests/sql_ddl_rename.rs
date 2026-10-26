use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn new_catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-rename-{}-{}",
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
fn renames_metadata_without_changing_other_files() {
    let dir = new_catalog();
    let created = ddl("CREATE TABLE events (id UInt64, name String)", &dir);
    assert!(created.status.success(), "{created:?}");
    fs::write(dir.join("part.bin"), b"leave me alone").unwrap();

    let renamed = ddl("RENAME TABLE events TO archived_events", &dir);
    assert!(renamed.status.success(), "{renamed:?}");
    assert!(!dir.join("events.schema").exists());
    assert_eq!(
        fs::read_to_string(dir.join("archived_events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\nname\tString\n"
    );
    assert_eq!(fs::read(dir.join("part.bin")).unwrap(), b"leave me alone");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_missing_source_and_existing_destination_without_data_loss() {
    let dir = new_catalog();
    let absent = ddl("RENAME TABLE unknown TO next", &dir);
    assert!(!absent.status.success());
    assert!(!dir.exists());

    let first = ddl("CREATE TABLE one (id UInt64)", &dir);
    assert!(first.status.success(), "{first:?}");
    let second = ddl("CREATE TABLE two (n Int64)", &dir);
    assert!(second.status.success(), "{second:?}");

    let collision = ddl("RENAME TABLE one TO two", &dir);
    assert!(!collision.status.success());
    assert!(String::from_utf8_lossy(&collision.stderr).contains("already exists"));
    assert_eq!(
        fs::read_to_string(dir.join("one.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("two.schema")).unwrap(),
        "rusthouse-schema-v1\nn\tInt64\n"
    );

    let identical = ddl("RENAME TABLE one TO one", &dir);
    assert!(!identical.status.success());
    fs::remove_dir_all(dir).unwrap();
}
