use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-add-hardened-{}-{}",
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
fn add_nested_column_keeps_a_versioned_schema_without_temp_files() {
    let dir = catalog();
    let created = sql("CREATE TABLE nested (id UInt64)", &dir);
    assert!(created.status.success(), "{created:?}");

    let added = sql(
        "ALTER TABLE nested ADD COLUMN detail Nullable(Array(UInt128))",
        &dir,
    );
    assert!(added.status.success(), "{added:?}");
    let result = sql("DESCRIBE TABLE nested", &dir);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "name\ttype\nid\tUInt64\ndetail\tNullable(Array(UInt128))\n"
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn add_does_not_modify_oversized_or_non_utf8_schemas() {
    let dir = catalog();
    fs::create_dir_all(&dir).unwrap();

    let oversized = vec![b'X'; 32 * 1024 + 1];
    fs::write(dir.join("big.schema"), &oversized).unwrap();
    let too_large = sql("ALTER TABLE big ADD COLUMN age UInt64", &dir);
    assert!(!too_large.status.success());
    assert!(String::from_utf8_lossy(&too_large.stderr).contains("schema is too large"));
    assert_eq!(fs::read(dir.join("big.schema")).unwrap(), oversized);

    fs::write(dir.join("bad.schema"), b"rusthouse-schema-v1\n\xff\n").unwrap();
    let invalid = sql("ALTER TABLE bad ADD COLUMN age UInt64", &dir);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid UTF-8"));
    fs::remove_dir_all(dir).unwrap();
}
