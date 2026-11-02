use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn run(database: &Path, sql: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn canonical_tpch_q06_runs_on_unscaled_float_columns() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-canonical-q06-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&database).unwrap();
    for statement in [
        "CREATE TABLE lineitem (l_extendedprice Float64, l_discount Float64, l_shipdate Date, l_quantity Float64)",
        "INSERT INTO lineitem VALUES (100, 0.05, '1994-01-05', 10), (50, 0.06, '1994-12-30', 20), (30, 0.07, '1995-01-01', 10), (20, 0.08, '1994-05-01', 10), (40, 0.07, '1994-03-01', 24)",
    ] {
        let result = run(&database, statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    // Keep this test tied to the exact canonical workload SQL, not a rewrite.
    // The benchmark runner removes the copyright comments and trailing ';'.
    let canonical = include_str!("../../../bench/tpch/queries/q06.sql");
    let sql = canonical
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    let sql = sql.trim().trim_end_matches(';');
    let result = run(&database, sql);
    assert!(
        result.status.success(),
        "canonical TPC-H Q6 failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"revenue\n8\n");
    fs::remove_dir_all(database).unwrap();
}
