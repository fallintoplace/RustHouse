use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn memory_bounded_hash_build_handles_dimension_above_old_65536_row_cap() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-80k-build-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimension (id UInt64)",
        "INSERT INTO facts SELECT number FROM numbers(40000)",
        "INSERT INTO facts SELECT number FROM numbers(40000, 40000)",
        "INSERT INTO dimension SELECT number FROM numbers(40000)",
        "INSERT INTO dimension SELECT number FROM numbers(40000, 40000)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT f.id FROM facts f INNER JOIN dimension d ON f.id = d.id LIMIT 1 OFFSET 79999",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"f.id\n79999\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn inner_join_builds_forty_thousand_rows_when_both_tables_are_large() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-large-build-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimension (id UInt64)",
        "INSERT INTO facts SELECT number FROM numbers(40000)",
        "INSERT INTO dimension SELECT number FROM numbers(40000)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT f.id FROM facts f INNER JOIN dimension d ON f.id = d.id LIMIT 1",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"f.id\n0\n");
    fs::remove_dir_all(db).unwrap();
}
