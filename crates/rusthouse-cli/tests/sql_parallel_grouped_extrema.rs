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
fn parallel_grouped_min_max_reduce_cross_partition_extrema() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-extrema-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (region UInt8, amount Int64, flag UInt8)", &db);
    for index in 0..40i64 {
        let region = index % 4;
        let value = -10000000000i64 + index * 1000000000;
        sql(&format!(
            "INSERT INTO facts VALUES ({region}, {value}, 1), ({region}, {}, 0)",
            value + 100_000_000_000
        ), &db);
    }
    assert_eq!(
        sql("SELECT region, min(amount) FROM facts WHERE flag = 1 GROUP BY region ORDER BY region", &db),
        "region\tmin(amount)\n0\t-10000000000\n1\t-9000000000\n2\t-8000000000\n3\t-7000000000\n"
    );
    assert_eq!(
        sql("SELECT region, max(amount) FROM facts WHERE flag = 1 GROUP BY region ORDER BY region", &db),
        "region\tmax(amount)\n0\t26000000000\n1\t27000000000\n2\t28000000000\n3\t29000000000\n"
    );
    fs::remove_dir_all(db).unwrap();
}
