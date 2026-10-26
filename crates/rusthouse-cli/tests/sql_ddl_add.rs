use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn new_catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-add-{}-{}",
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
fn adds_columns_durably_and_preserves_previous_schema() {
    let dir = new_catalog();
    assert!(
        ddl("CREATE TABLE events (id UInt64)", &dir)
            .status
            .success()
    );

    let added = ddl("ALTER TABLE events ADD COLUMN info Nullable(String)", &dir);
    assert!(added.status.success(), "{added:?}");
    assert_eq!(
        fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\ninfo\tNullable(String)\n"
    );

    let duplicate = ddl("ALTER TABLE events ADD COLUMN info UInt8", &dir);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already exists"));

    let skipped = ddl(
        "ALTER TABLE events ADD COLUMN IF NOT EXISTS info UInt8",
        &dir,
    );
    assert!(skipped.status.success(), "{skipped:?}");
    assert_eq!(
        fs::read_to_string(dir.join("events.schema")).unwrap(),
        "rusthouse-schema-v1\nid\tUInt64\ninfo\tNullable(String)\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_corrupt_catalogs_and_missing_tables_without_clobbering() {
    let dir = new_catalog();
    let missing = ddl("ALTER TABLE no_table ADD COLUMN age UInt8", &dir);
    assert!(!missing.status.success());
    assert!(!dir.exists());

    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("events.schema"), b"corrupt format\n").unwrap();
    let result = ddl("ALTER TABLE events ADD COLUMN age UInt8", &dir);
    assert!(!result.status.success());
    assert_eq!(
        fs::read(dir.join("events.schema")).unwrap(),
        b"corrupt format\n"
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

    fs::remove_dir_all(dir).unwrap();
}
