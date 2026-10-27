use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn query(dir: &Path, statement: &str) -> Vec<u8> {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(dir)
        .output()
        .unwrap();
    assert!(result.status.success(), "query failed: {result:?}");
    result.stdout
}

#[test]
fn dates_and_timestamps_round_trip_and_filter_correctly() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-temporal-roundtrip-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    query(&dir, "CREATE TABLE events (d Date, ts DateTime)");
    query(
        &dir,
        "INSERT INTO events VALUES ('2013-07-01', '2013-07-01 12:34:56'), ('2013-07-14', '2013-07-14 23:59:59')",
    );
    assert_eq!(
        query(&dir, "SELECT d, ts FROM events ORDER BY d"),
        b"d\tts\n2013-07-01\t2013-07-01 12:34:56\n2013-07-14\t2013-07-14 23:59:59\n"
    );
    assert_eq!(
        query(&dir, "SELECT count(*) FROM events WHERE d >= '2013-07-14'"),
        b"count\n1\n"
    );
    assert_eq!(
        query(&dir, "SELECT max(d) FROM events"),
        b"max(d)\n2013-07-14\n"
    );
    assert_eq!(
        query(&dir, "SELECT min(ts) FROM events"),
        b"min(ts)\n2013-07-01 12:34:56\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn q6_style_revenue_filters_use_folded_constants() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-q6-interval-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    query(
        &dir,
        "CREATE TABLE lineitem (l_shipdate Date, l_extendedprice Float64, l_discount Float64, l_quantity UInt64)",
    );
    query(
        &dir,
        "INSERT INTO lineitem VALUES \
         ('1994-06-01', 100, 0.06, 10), \
         ('1994-02-01', 100, 0.05, 10), \
         ('1994-02-01', 100, 0.07, 10), \
         ('1994-02-01', 100, 0.04, 10), \
         ('1994-02-01', 100, 0.06, 30), \
         ('1995-02-01', 100, 0.06, 10)",
    );
    let output = query(
        &dir,
        "SELECT SUM(l_extendedprice * l_discount) AS revenue \
         FROM lineitem WHERE l_shipdate >= DATE '1994-01-01' \
         AND l_shipdate < DATE '1994-01-01' + INTERVAL '1' YEAR \
         AND l_discount BETWEEN 0.06 - 0.01 AND 0.06 + 0.01 \
         AND l_quantity < 24",
    );
    let output = String::from_utf8(output).unwrap();
    let mut lines = output.lines();
    assert_eq!(lines.next(), Some("revenue"));
    let revenue: f64 = lines.next().unwrap().parse().unwrap();
    assert!((revenue - 18.0).abs() < 1e-9, "{output}");
    assert_eq!(lines.next(), None);
    std::fs::remove_dir_all(dir).unwrap();
}
