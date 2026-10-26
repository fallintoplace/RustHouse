use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn directory() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-show-like-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(statement: &str, directory: &Path, mode: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, mode])
        .arg(directory)
        .output()
        .unwrap()
}

#[test]
fn show_tables_like_filters_catalog_without_changing_table_state() {
    let catalog = directory();
    for table in ["orders_1", "ordersX2", "other"] {
        let result = sql(
            &format!("CREATE TABLE {table} (id UInt64)"),
            &catalog,
            "--catalog",
        );
        assert!(result.status.success(), "{result:?}");
    }
    let result = sql(r"SHOW TABLES LIKE 'orders\_%'", &catalog, "--catalog");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"name\norders_1\n");
    let all = sql("SHOW TABLES", &catalog, "--catalog");
    assert!(all.status.success(), "{all:?}");
    assert_eq!(all.stdout, b"name\nordersX2\norders_1\nother\n");
    let invalid = sql("SHOW TABLES LIKE 2", &catalog, "--catalog");
    assert!(!invalid.status.success());
    fs::remove_dir_all(catalog).unwrap();
}

#[test]
fn show_tables_like_works_in_persistent_database_mode() {
    let database = directory();
    for table in ["events_a", "events_b", "metrics"] {
        let result = sql(
            &format!("CREATE TABLE {table} (id UInt64)"),
            &database,
            "--database",
        );
        assert!(result.status.success(), "{result:?}");
    }
    let result = sql("SHOW TABLES LIKE 'events_%'", &database, "--database");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"name\nevents_a\nevents_b\n");
    fs::remove_dir_all(database).unwrap();
}
