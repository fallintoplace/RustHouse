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
fn weighted_grouped_average_reduces_partial_counts_not_partial_means() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-weighted-avg-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (region UInt8, amount Int64, flag UInt8)", &db);
    for index in 0..40 {
        let group = index % 4;
        sql(&format!(
            "INSERT INTO facts VALUES ({group}, 0, 1), ({group}, 6, 0), ({group}, 6, 0)"
        ), &db);
    }
    assert_eq!(
        sql("SELECT region, avg(amount) FROM facts GROUP BY region ORDER BY region", &db),
        "region\tavg(amount)\n0\t4\n1\t4\n2\t4\n3\t4\n"
    );
    assert_eq!(
        sql("SELECT region, avg(amount) FROM facts WHERE flag = 1 GROUP BY region ORDER BY region", &db),
        "region\tavg(amount)\n0\t0\n1\t0\n2\t0\n3\t0\n"
    );
    fs::remove_dir_all(db).unwrap();
}
