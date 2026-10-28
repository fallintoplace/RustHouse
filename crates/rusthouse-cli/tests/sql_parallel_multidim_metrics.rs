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
fn parallel_multi_dimension_metrics_combine_all_shards() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-multi-dim-metrics-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE sales (region UInt8, sku String, revenue Int64, flag UInt8)", &db);
    for index in 0..40 {
        let region = index % 4;
        let sku = if region % 2 == 0 { "A" } else { "B" };
        sql(&format!(
            "INSERT INTO sales VALUES ({region}, '{sku}', 5000000000, 1), ({region}, '{sku}', -1000000000, 0)"
        ), &db);
    }
    assert_eq!(
        sql("SELECT region, sku, sum(revenue), count(revenue) FROM sales GROUP BY region, sku ORDER BY region, sku", &db),
        "region\tsku\tsum(revenue)\tcount(revenue)\n0\tA\t40000000000\t20\n1\tB\t40000000000\t20\n2\tA\t40000000000\t20\n3\tB\t40000000000\t20\n"
    );
    assert_eq!(
        sql("SELECT region, sku, sum(revenue), count(revenue) FROM sales WHERE flag = 1 GROUP BY region, sku ORDER BY region, sku", &db),
        "region\tsku\tsum(revenue)\tcount(revenue)\n0\tA\t50000000000\t10\n1\tB\t50000000000\t10\n2\tA\t50000000000\t10\n3\tB\t50000000000\t10\n"
    );
    fs::remove_dir_all(db).unwrap();
}
