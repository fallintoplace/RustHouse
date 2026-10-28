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
fn grouped_multi_metric_analytics_merge_parallel_sums_exactly() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-metrics-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE events (region UInt16, revenue Int64, cost Int64, flag UInt8)", &db);
    for index in 0..40 {
        let region = index % 4;
        sql(&format!(
            "INSERT INTO events VALUES ({region}, 5000000000, -2000000000, 1), ({region}, -1000000000, 3000000000, 0)"
        ), &db);
    }
    assert_eq!(
        sql("SELECT region, sum(revenue), sum(cost) FROM events GROUP BY region ORDER BY region", &db),
        "region\tsum(revenue)\tsum(cost)\n0\t40000000000\t10000000000\n1\t40000000000\t10000000000\n2\t40000000000\t10000000000\n3\t40000000000\t10000000000\n"
    );
    assert_eq!(
        sql("SELECT region, sum(revenue), sum(cost) FROM events WHERE flag = 1 GROUP BY region ORDER BY region", &db),
        "region\tsum(revenue)\tsum(cost)\n0\t50000000000\t-20000000000\n1\t50000000000\t-20000000000\n2\t50000000000\t-20000000000\n3\t50000000000\t-20000000000\n"
    );
    fs::remove_dir_all(db).unwrap();
}
