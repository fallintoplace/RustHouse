use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-grouped-quantile-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
fn ok(db: &Path, sql: &str) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{sql}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
#[test]
fn high_cardinality_facts_retain_only_grouped_latency_samples() {
    let db = database();
    ok(&db, "CREATE TABLE events (region String, latency UInt64)");
    let tsv = db.join("events.tsv");
    let mut content = String::from("region\tlatency\n");
    for value in 0..30_000u64 {
        use std::fmt::Write as _;
        writeln!(&mut content, "west\t{value}").unwrap();
        writeln!(&mut content, "east\t{}", value + 100_000).unwrap();
    }
    fs::write(&tsv, content).unwrap();
    ok(&db, &format!("COPY events FROM '{}' WITH (FORMAT TSV, HEADER TRUE)", tsv.display()));
    let output = ok(
        &db,
        "SELECT region, quantile(0.95)(latency) FROM events GROUP BY region ORDER BY 2 DESC",
    );
    let rows = output.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 3, "{output}");
    assert_eq!(rows[0], "region\tquantile(0.95)(latency)");
    assert!(rows[1].starts_with("east\t"), "{output}");
    let east = rows[1].split('\t').nth(1).unwrap().parse::<u64>().unwrap();
    let west = rows[2].split('\t').nth(1).unwrap().parse::<u64>().unwrap();
    assert!(east.abs_diff(128500) < 3000, "{east}");
    assert!(west.abs_diff(28500) < 3000, "{west}");
    let filtered = ok(
        &db,
        "SELECT region, quantile(0.95)(latency) FROM events GROUP BY region HAVING quantile(0.95)(latency) > 100000 ORDER BY region",
    );
    assert_eq!(filtered.lines().count(), 2);
    fs::remove_dir_all(db).unwrap();
}
#[test]
fn all_null_cohorts_report_null_percentile() {
    let db = database();
    ok(&db, "CREATE TABLE events (region String, latency Nullable(UInt64))");
    ok(&db, "INSERT INTO events VALUES ('west', 1), ('west', 3), ('east', NULL)");
    assert_eq!(
        ok(&db, "SELECT region, quantile(0.5)(latency) FROM events GROUP BY region ORDER BY region"),
        "region\tquantile(0.5)(latency)\neast\tNULL\nwest\t3\n"
    );
    fs::remove_dir_all(db).unwrap();
}
