//! A small, deterministic revenue-filter workload using scaled integers.
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn run(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

fn successful(statement: &str, database: &Path) -> Output {
    let result = run(statement, database);
    assert!(result.status.success(), "{statement}: {result:?}");
    result
}

#[test]
fn computes_revenue_with_inclusive_discount_and_exclusive_date_boundaries() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-revenue-filter-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&database).unwrap();
    successful(
        "CREATE TABLE lineitem (l_shipdate Date, l_extendedprice UInt64, l_discount UInt64, l_quantity UInt64)",
        &database,
    );
    successful(
        "INSERT INTO lineitem VALUES         ('1994-03-01', 10000, 500, 10),         ('1994-12-31', 20000, 700, 23),         ('1994-01-01', 30000, 600, 24),         ('1995-01-01', 40000, 500, 10),         ('1993-12-31', 50000, 500, 10),         ('1994-06-01', 60000, 400, 10),         ('1994-06-01', 70000, 800, 10)",
        &database,
    );

    let result = successful(
        "SELECT SUM(l_extendedprice * l_discount) AS revenue_bps,         COUNT(*) AS matched_rows FROM lineitem         WHERE l_shipdate >= '1994-01-01'         AND l_shipdate < '1995-01-01'         AND l_discount BETWEEN 500 AND 700         AND l_quantity < 24",
        &database,
    );
    assert_eq!(result.stdout, b"revenue_bps\tmatched_rows\n19000000\t2\n");

    // The integer result is in price-minor-units times discount basis points.
    // Do not compare this scaled result to a floating-point SQL DECIMAL result.
    fs::remove_dir_all(database).unwrap();
}
