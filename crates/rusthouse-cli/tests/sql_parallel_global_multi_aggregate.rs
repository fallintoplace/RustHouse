use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, db: &Path) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"]).arg(db).output().unwrap();
    assert!(result.status.success(), "{statement}: {result:?}");
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn parallel_multi_aggregates_scan_each_immutable_part_once() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-global-metrics-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (amount Int64, cost Int64, flag UInt8)", &db);
    for _ in 0..40 {
        sql("INSERT INTO facts VALUES (5000000000, -2000000000, 1), (-1000000000, 3000000000, 0)", &db);
    }
    assert_eq!(
        sql("SELECT count(*), sum(amount), sum(cost) FROM facts", &db),
        "count(*)\tsum(amount)\tsum(cost)\n80\t160000000000\t40000000000\n"
    );
    assert_eq!(
        sql("SELECT count(*), sum(amount), sum(cost) FROM facts WHERE flag = 1", &db),
        "count(*)\tsum(amount)\tsum(cost)\n40\t200000000000\t-80000000000\n"
    );
    fs::remove_dir_all(db).unwrap();
}
