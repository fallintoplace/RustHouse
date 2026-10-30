#![cfg(debug_assertions)]
use std::fs;
use std::process::Command;

#[test]
fn interrupted_catalog_rename_keeps_a_durable_table_name() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-catalog-rename-crash-{}", std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    let create = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "CREATE TABLE events (id UInt64)", "--catalog"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(create.status.success(), "{create:?}");

    let crash = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "RENAME TABLE events TO archive", "--catalog"])
        .arg(&path)
        .env("RUSTHOUSE_DEBUG_CRASH_AT", "catalog-rename-after-link")
        .output()
        .unwrap();
    assert_eq!(crash.status.code(), Some(86), "{crash:?}");
    assert!(path.join("events.schema").exists());
    assert!(path.join("archive.schema").exists());
    assert_eq!(
        fs::read(path.join("archive.schema")).unwrap(),
        fs::read(path.join("events.schema")).unwrap()
    );
    fs::remove_dir_all(path).unwrap();
}
