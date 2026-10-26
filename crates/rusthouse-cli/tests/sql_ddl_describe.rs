use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-describe-{}-{}",
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
fn describe_table_lists_persisted_types_in_schema_order() {
    let dir = catalog();
    let created = sql(
        "CREATE TABLE readings (ts DateTime, value Float64, tags Array(String))",
        &dir,
    );
    assert!(created.status.success(), "{created:?}");
    let result = sql("DESCRIBE TABLE readings;", &dir);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "name\ttype\nts\tDateTime\nvalue\tFloat64\ntags\tArray(String)\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn describe_table_rejects_corrupt_and_missing_schemas() {
    let dir = catalog();
    let missing = sql("DESCRIBE TABLE unknown", &dir);
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(!dir.exists());

    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("corrupt.schema"), "bad schema\n").unwrap();
    let corrupt = sql("DESCRIBE TABLE corrupt", &dir);
    assert!(!corrupt.status.success());
    assert!(corrupt.stdout.is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn describe_table_rejects_extra_sql() {
    let dir = catalog();
    let bad = sql("DESCRIBE TABLE events; DROP TABLE events", &dir);
    assert!(!bad.status.success());
    assert!(!dir.exists());
}
