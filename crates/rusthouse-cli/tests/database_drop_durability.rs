use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn interrupted_drop_retains_parts_after_durable_schema_hide() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-drop-tombstone-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&dir)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (id UInt64)",
        "INSERT INTO events VALUES (42)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let interrupted = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "DROP TABLE events", "--database"])
        .arg(&dir)
        .env("RUSTHOUSE_DEBUG_CRASH_AT", "drop-after-schema-hide")
        .output()
        .unwrap();
    assert_eq!(interrupted.status.code(), Some(86), "{interrupted:?}");
    assert!(!dir.join("events.schema").exists());
    assert!(dir.join("events.parts").is_dir());
    assert!(fs::read_dir(&dir).unwrap().any(|entry| {
        entry.unwrap().file_name().to_string_lossy().starts_with(".events.drop-")
    }));
    fs::remove_dir_all(dir).unwrap();
}
