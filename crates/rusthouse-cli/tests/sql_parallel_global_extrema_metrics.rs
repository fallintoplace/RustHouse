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
fn parallel_multi_metric_scan_merges_global_extrema_without_row_loss() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-global-extrema-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (amount Int64, flag UInt8)", &db);
    for index in 0..40i64 {
        let amount = -20000000000i64 + index * 1000000000;
        sql(&format!(
            "INSERT INTO facts VALUES ({amount}, 1), ({}, 0)",
            amount + 100000000000
        ), &db);
    }
    assert_eq!(
        sql("SELECT count(*), min(amount), max(amount) FROM facts WHERE flag = 1", &db),
        "count(*)\tmin(amount)\tmax(amount)\n40\t-20000000000\t19000000000\n"
    );
    assert_eq!(
        sql("SELECT count(*), min(amount), max(amount) FROM facts", &db),
        "count(*)\tmin(amount)\tmax(amount)\n80\t-20000000000\t119000000000\n"
    );
    fs::remove_dir_all(db).unwrap();
}
