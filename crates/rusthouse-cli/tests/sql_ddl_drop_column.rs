use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn new_catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-drop-column-{}-{}",
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
fn removes_columns_without_changing_other_metadata() {
    let dir = new_catalog();
    assert!(
        ddl(
            "CREATE TABLE events (id UInt64, age UInt8, payload String)",
            &dir
        )
        .status
        .success()
    );
    let result = ddl("ALTER TABLE events DROP COLUMN age", &dir);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\npayload\tString\n"
    );

    let missing = ddl("ALTER TABLE events DROP COLUMN age", &dir);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("unknown column"));

    let skipped = ddl("ALTER TABLE events DROP COLUMN IF EXISTS age", &dir);
    assert!(skipped.status.success(), "{skipped:?}");
    assert_eq!(
        fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\npayload\tString\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_dropping_last_column_and_missing_tables() {
    let dir = new_catalog();
    let nonexistent = ddl("ALTER TABLE missing DROP COLUMN id", &dir);
    assert!(!nonexistent.status.success());
    assert!(!dir.exists());

    let created = ddl("CREATE TABLE events (id UInt64)", &dir);
    assert!(created.status.success(), "{created:?}");
    let last = ddl("ALTER TABLE events DROP COLUMN id", &dir);
    assert!(!last.status.success());
    assert!(String::from_utf8_lossy(&last.stderr).contains("last column"));
    assert_eq!(
        fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\n"
    );
    fs::remove_dir_all(dir).unwrap();
}
