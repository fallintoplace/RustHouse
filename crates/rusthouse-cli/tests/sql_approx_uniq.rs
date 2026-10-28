//! SQL uniq() over generated billion-row sources and physical typed parts.
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-uniq-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn sql(db: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

fn ok(db: &Path, statement: &str) -> String {
    let result = sql(db, statement);
    assert!(
        result.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn billion_row_virtual_series_does_not_iterate_or_allocate_keys() {
    let db = database();
    assert_eq!(
        ok(&db, "SELECT uniq(number) FROM numbers(1000000000)"),
        "uniq(number)\n1000000000\n"
    );
    assert_eq!(
        ok(&db, "SELECT uniq(number) FROM numbers(1000000000) WHERE number < 1000"),
        "uniq(number)\n1000\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn stored_high_cardinality_distinct_estimates_in_fixed_memory() {
    let db = database();
    ok(&db, "CREATE TABLE events (id UInt64)");
    ok(&db, "INSERT INTO events SELECT number FROM numbers(30000)");
    let text = ok(&db, "SELECT uniq(id) FROM events");
    let estimate = text
        .strip_prefix("uniq(id)\n")
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap();
    assert!(estimate.abs_diff(30000) < 1500, "{estimate}");
    assert!(ok(&db, "EXPLAIN SELECT uniq(id) FROM events").contains("uniq(id)"));
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn null_string_values_do_not_contribute_cardinality() {
    let db = database();
    ok(&db, "CREATE TABLE texts (id UInt64, tag Nullable(String))");
    ok(&db, "INSERT INTO texts VALUES (1, 'blue'), (2, NULL), (3, 'blue'), (4, 'red')");
    assert_eq!(ok(&db, "SELECT uniq(tag) FROM texts"), "uniq(tag)\n2\n");
    fs::remove_dir_all(db).unwrap();
}
