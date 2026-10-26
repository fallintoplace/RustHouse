use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-rename-column-{}-{}",
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
fn renames_catalog_columns_without_touching_data() {
    let dir = catalog();
    assert!(
        ddl("CREATE TABLE events (id UInt64, payload String)", &dir)
            .status
            .success()
    );
    fs::write(dir.join("part.bin"), b"keep data").unwrap();

    let renamed = ddl("ALTER TABLE events RENAME COLUMN payload TO message", &dir);
    assert!(renamed.status.success(), "{renamed:?}");
    assert_eq!(
        fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\nmessage\tString\n"
    );
    assert_eq!(fs::read(dir.join("part.bin")).unwrap(), b"keep data");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refuses_renaming_missing_or_duplicate_columns_without_mutation() {
    let dir = catalog();
    assert!(
        ddl("CREATE TABLE t (a UInt64, b String)", &dir)
            .status
            .success()
    );
    let before = fs::read(dir.join("t.schema")).unwrap();
    for sql in [
        "ALTER TABLE t RENAME COLUMN missing TO next",
        "ALTER TABLE t RENAME COLUMN a TO b",
        "ALTER TABLE t RENAME COLUMN a TO a",
    ] {
        let result = ddl(sql, &dir);
        assert!(!result.status.success(), "{sql}");
        assert_eq!(fs::read(dir.join("t.schema")).unwrap(), before);
    }
    fs::remove_dir_all(dir).unwrap();
}
