use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-modify-column-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn ddl(sql: &str, dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql, "--catalog"])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn modifies_catalog_type_without_changing_data_or_column_order() {
    let dir = catalog();
    assert!(ddl("CREATE TABLE events (id UInt64, payload String)", &dir).status.success());
    fs::write(dir.join("part.bin"), b"untouched").unwrap();

    let changed = ddl("ALTER TABLE events MODIFY COLUMN payload Nullable(String)", &dir);
    assert!(changed.status.success(), "{changed:?}");
    let schema = "rusthouse-schema-v1\nid\tUInt64\npayload\tNullable(String)\n";
    assert_eq!(fs::read_to_string(dir.join("events.schema")).unwrap(), schema);
    assert_eq!(fs::read(dir.join("part.bin")).unwrap(), b"untouched");
    assert!(ddl("ALTER TABLE events MODIFY COLUMN payload Nullable(String)", &dir).status.success());
    assert_eq!(fs::read_to_string(dir.join("events.schema")).unwrap(), schema);

    let described = ddl("DESCRIBE TABLE events", &dir);
    assert!(described.status.success());
    assert!(String::from_utf8(described.stdout).unwrap().contains("payload\tNullable(String)"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_invalid_type_and_missing_column_without_mutation() {
    let dir = catalog();
    assert!(ddl("CREATE TABLE events (id UInt64, payload String)", &dir).status.success());
    let before = fs::read(dir.join("events.schema")).unwrap();
    for sql in [
        "ALTER TABLE events MODIFY COLUMN missing String",
        "ALTER TABLE events MODIFY COLUMN payload MysteryType",
        "ALTER TABLE events MODIFY payload Nullable(String)",
        "ALTER TABLE missing MODIFY COLUMN payload String",
    ] {
        let result = ddl(sql, &dir);
        assert!(!result.status.success(), "{sql}: {result:?}");
        assert_eq!(fs::read(dir.join("events.schema")).unwrap(), before);
    }
    fs::remove_dir_all(dir).unwrap();
}
