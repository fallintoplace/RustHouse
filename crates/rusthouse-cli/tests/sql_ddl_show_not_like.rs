use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn directory() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-not-like-{}-{}",
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
fn show_tables_not_like_filters_catalog_with_existing_like_semantics() {
    let catalog = directory();
    for table in ["events_1", "eventsX2", "metrics"] {
        let created = sql(
            &format!("CREATE TABLE {table} (id UInt64)"),
            &catalog,
            "--catalog",
        );
        assert!(created.status.success(), "{created:?}");
    }

    let filtered = sql(r"SHOW TABLES NOT LIKE 'events\_%'", &catalog, "--catalog");
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(filtered.stdout, b"name\neventsX2\nmetrics\n");

    let all_excluded = sql("SHOW TABLES NOT LIKE '%'", &catalog, "--catalog");
    assert!(all_excluded.status.success(), "{all_excluded:?}");
    assert_eq!(all_excluded.stdout, b"name\n");

    let original = sql("SHOW TABLES", &catalog, "--catalog");
    assert_eq!(original.stdout, b"name\neventsX2\nevents_1\nmetrics\n");
    fs::remove_dir_all(catalog).unwrap();
}

#[test]
fn show_tables_not_like_works_in_database_mode_and_missing_catalog() {
    let database = directory();
    let missing = sql("SHOW TABLES NOT LIKE '%'", &database, "--database");
    assert!(missing.status.success(), "{missing:?}");
    assert_eq!(missing.stdout, b"name\n");
    assert!(!database.exists());

    for table in ["logs_1", "users"] {
        let created = sql(
            &format!("CREATE TABLE {table} (id UInt64)"),
            &database,
            "--database",
        );
        assert!(created.status.success(), "{created:?}");
    }
    let result = sql("SHOW TABLES NOT LIKE 'logs%'", &database, "--database");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"name\nusers\n");

    let invalid = sql("SHOW TABLES NOT LIKE 7", &database, "--database");
    assert!(!invalid.status.success());
    fs::remove_dir_all(database).unwrap();
}
