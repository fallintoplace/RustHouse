use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-quantile-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn ok(db: &Path, statement: &str) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn billion_row_generated_percentiles_are_constant_time() {
    let db = database();
    assert_eq!(
        ok(&db, "SELECT quantile(0.95)(number) FROM numbers(1000000000)"),
        "quantile(0.95)(number)\n949999999\n"
    );
    assert_eq!(
        ok(&db, "SELECT quantile(0.5)(number) FROM numbers(1000000000) WHERE number >= 999900000"),
        "quantile(0.5)(number)\n999950000\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn physical_numeric_quantile_uses_fixed_size_reservoir() {
    let db = database();
    ok(&db, "CREATE TABLE events (latency UInt64)");
    ok(&db, "INSERT INTO events SELECT number FROM numbers(30000)");
    let result = ok(&db, "SELECT quantile(0.95)(latency) FROM events");
    let value = result
        .strip_prefix("quantile(0.95)(latency)\n")
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap();
    assert!(value.abs_diff(28_500) < 1500, "{value}");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn nullable_quantile_ignores_nulls_and_filters_on_text() {
    let db = database();
    ok(&db, "CREATE TABLE events (latency Nullable(UInt64), region String)");
    ok(&db, "INSERT INTO events VALUES (100, 'north'), (NULL, 'north'), (200, 'north'), (900, 'south')");
    assert_eq!(
        ok(&db, "SELECT quantile(0.5)(latency) FROM events WHERE region = 'north'"),
        "quantile(0.5)(latency)\n200\n"
    );
    assert_eq!(
        ok(&db, "SELECT quantile(0.95)(latency) FROM events WHERE region = 'missing'"),
        "quantile(0.95)(latency)\nNULL\n"
    );
    fs::remove_dir_all(db).unwrap();
}
