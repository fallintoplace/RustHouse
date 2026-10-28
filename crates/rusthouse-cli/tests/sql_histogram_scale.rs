use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-histogram-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
fn ok(db: &Path, statement: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn billion_generated_rows_form_exact_distribution_in_constant_time() {
    let db = database();
    let result = ok(&db, "SELECT histogram(10)(number) FROM numbers(1000000000)");
    assert!(result.starts_with("histogram(10)(number)\n["));
    assert_eq!(result.matches(",100000000)").count(), 10);
    assert!(result.contains("(0,99999999,100000000)"));
    assert!(result.contains("(900000000,999999999,100000000)"));
    fs::remove_dir_all(db).unwrap();
}
#[test]
fn physical_fact_histogram_has_bounded_two_pass_state() {
    let db = database();
    ok(&db, "CREATE TABLE events (latency UInt64)");
    ok(&db, "INSERT INTO events SELECT number FROM numbers(50000)");
    let output = ok(&db, "SELECT histogram(5)(latency) FROM events");
    assert!(output.starts_with("histogram(5)(latency)\n["));
    assert_eq!(output.matches(",10000)").count(), 5, "{output}");
    fs::remove_dir_all(db).unwrap();
}
#[test]
fn nullable_filtered_histogram_skips_null_values() {
    let db = database();
    ok(&db, "CREATE TABLE events (region String, latency Nullable(UInt64))");
    ok(&db, "INSERT INTO events VALUES ('west', 10), ('west', NULL), ('west', 20), ('east', 80)");
    let result = ok(&db, "SELECT histogram(2)(latency) FROM events WHERE region = 'west'");
    assert!(result.contains("(10,15,1)"), "{result}");
    assert!(result.contains("(15,20,1)"), "{result}");
    fs::remove_dir_all(db).unwrap();
}
