use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-binary-stats-{}-{}",
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
fn correlation_of_two_large_fact_columns_needs_no_sorting() {
    let db = database();
    ok(&db, "CREATE TABLE facts (x UInt64, y UInt64)");
    let input = db.join("facts.tsv");
    let mut content = String::from("x\ty\n");
    for index in 0..60_000u64 {
        use std::fmt::Write as _;
        writeln!(&mut content, "{index}\t{}", 3 * index + 7).unwrap();
    }
    fs::write(&input, content).unwrap();
    ok(&db, &format!("COPY facts FROM '{}' WITH (FORMAT TSV, HEADER TRUE)", input.display()));
    let correlation = ok(&db, "SELECT corr(x,y) FROM facts");
    let result = correlation.lines().nth(1).unwrap().parse::<f64>().unwrap();
    assert!((result - 1.0).abs() < 1e-10);
    let covariance = ok(&db, "SELECT covarPop(x,y) FROM facts");
    assert!(covariance.lines().nth(1).unwrap().parse::<f64>().unwrap() > 800_000_000.0);
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn pairwise_nulls_and_where_exclude_incomplete_measurements() {
    let db = database();
    ok(&db, "CREATE TABLE facts (x Nullable(UInt64), y Nullable(UInt64), region String)");
    ok(&db, "INSERT INTO facts VALUES (1, 2, 'west'), (2, 4, 'west'), (NULL, 1000, 'west'), (7, NULL, 'west'), (5, 900, 'east')");
    assert_eq!(ok(&db, "SELECT corr(x,y) FROM facts WHERE region = 'west'"), "corr(x,y)\n1\n");
    assert_eq!(ok(&db, "SELECT covarPop(x,y) FROM facts WHERE region = 'west'"), "covarPop(x,y)\n0.5\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn billion_row_generated_correlation_uses_analytic_moments() {
    let db = database();
    assert_eq!(
        ok(&db, "SELECT corr(number,number) FROM numbers(1000000000)"),
        "corr(number,number)\n1\n"
    );
    assert_eq!(
        ok(&db, "SELECT covarPop(number,number) FROM numbers(1)"),
        "covarPop(number,number)\n0\n"
    );
    fs::remove_dir_all(db).unwrap();
}
