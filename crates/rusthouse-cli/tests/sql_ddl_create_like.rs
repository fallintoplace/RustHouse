use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn schema_only_catalog_supports_create_table_like() {
    let catalog = std::env::temp_dir().join(format!(
        "rusthouse-schema-clone-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--catalog"])
            .arg(&catalog)
            .output()
            .unwrap()
    };
    assert!(
        sql("CREATE TABLE source (id UInt64, message Nullable(String))")
            .status
            .success()
    );
    assert!(sql("CREATE TABLE copied LIKE source").status.success());
    let described = sql("DESCRIBE TABLE copied");
    assert!(described.status.success(), "{described:?}");
    assert_eq!(
        described.stdout,
        b"name\ttype\nid\tUInt64\nmessage\tNullable(String)\n"
    );
    assert!(
        sql("CREATE TABLE IF NOT EXISTS copied LIKE source")
            .status
            .success()
    );
    assert!(!sql("CREATE TABLE copied LIKE source").status.success());
    assert!(!sql("CREATE TABLE missing LIKE unknown").status.success());
    fs::remove_dir_all(catalog).unwrap();
}
