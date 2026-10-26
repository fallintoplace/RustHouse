use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-show-create-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(query: &str, dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--catalog"])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn show_create_reconstructs_a_round_trippable_schema_after_alters() {
    let source = catalog();
    let destination = catalog();
    assert!(
        sql(
            "CREATE TABLE events (id UInt64, tags Array(Nullable(String)), active Bool)",
            &source
        )
        .status
        .success()
    );
    assert!(
        sql("ALTER TABLE events RENAME COLUMN active TO enabled", &source)
            .status
            .success()
    );

    let result = sql("SHOW CREATE TABLE events;", &source);
    assert!(result.status.success(), "{result:?}");
    let ddl = String::from_utf8(result.stdout).unwrap();
    assert_eq!(
        ddl,
        "CREATE TABLE events (\n    id UInt64,\n    tags Array(Nullable(String)),\n    enabled Bool\n);\n"
    );

    let recreated = sql(&ddl, &destination);
    assert!(recreated.status.success(), "{recreated:?}");
    assert_eq!(
        fs::read(source.join("events.schema")).unwrap(),
        fs::read(destination.join("events.schema")).unwrap()
    );
    fs::remove_dir_all(source).unwrap();
    fs::remove_dir_all(destination).unwrap();
}

#[test]
fn show_create_rejects_absent_and_corrupt_schemas_without_writing_files() {
    let dir = catalog();
    let missing = sql("SHOW CREATE TABLE missing", &dir);
    assert!(!missing.status.success());
    assert!(!dir.exists());

    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("bad.schema"), b"bad format\n").unwrap();
    let corrupt = sql("SHOW CREATE TABLE bad", &dir);
    assert!(!corrupt.status.success());
    assert_eq!(fs::read(dir.join("bad.schema")).unwrap(), b"bad format\n");
    assert!(!dir.join("missing.schema").exists());
    fs::remove_dir_all(dir).unwrap();
}
