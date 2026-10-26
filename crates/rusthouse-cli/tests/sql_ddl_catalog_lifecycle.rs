use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-lifecycle-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(query: &str, dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--catalog"])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn drop_table_updates_catalog_listing_without_touching_other_files() {
    let dir = catalog();
    for name in ["events", "archive"] {
        let created = sql(&format!("CREATE TABLE {name} (id UInt64)"), &dir);
        assert!(created.status.success(), "{created:?}");
    }
    let part = dir.join("real-part.bin");
    fs::write(&part, b"keep this part").unwrap();

    let listed = sql("SHOW TABLES", &dir);
    assert!(listed.status.success(), "{listed:?}");
    assert_eq!(
        String::from_utf8(listed.stdout).unwrap(),
        "name\narchive\nevents\n"
    );

    let dropped = sql("DROP TABLE events", &dir);
    assert!(dropped.status.success(), "{dropped:?}");
    assert!(!dir.join("events.schema").exists());
    assert!(dir.join("archive.schema").exists());
    assert_eq!(fs::read(&part).unwrap(), b"keep this part");

    let listed = sql("SHOW TABLES", &dir);
    assert_eq!(String::from_utf8(listed.stdout).unwrap(), "name\narchive\n");
    let missing = sql("DESCRIBE TABLE events", &dir);
    assert!(!missing.status.success());

    let ignored = sql("DROP TABLE IF EXISTS events", &dir);
    assert!(ignored.status.success(), "{ignored:?}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn drop_if_exists_does_not_create_an_absent_catalog() {
    let dir = catalog();
    let result = sql("DROP TABLE IF EXISTS missing", &dir);
    assert!(result.status.success(), "{result:?}");
    assert!(!dir.exists());
    let invalid = sql("DROP TABLE missing; SHOW TABLES", &dir);
    assert!(!invalid.status.success());
    assert!(!dir.exists());
}
