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
fn parallel_multidim_extrema_preserve_large_signed_value_order() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-multidim-extrema-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (region UInt8, sku String, amount Int64, flag UInt8)", &db);
    for index in 0..40i64 {
        let region = index % 4;
        let sku = if region % 2 == 0 { "A" } else { "B" };
        let value = -10000000000i64 + index * 1000000000;
        sql(&format!(
            "INSERT INTO facts VALUES ({region}, '{sku}', {value}, 1), ({region}, '{sku}', {}, 0)",
            value + 100000000000
        ), &db);
    }
    assert_eq!(
        sql("SELECT region, sku, min(amount), max(amount) FROM facts WHERE flag = 1 GROUP BY region, sku ORDER BY region, sku", &db),
        "region\tsku\tmin(amount)\tmax(amount)\n0\tA\t-10000000000\t26000000000\n1\tB\t-9000000000\t27000000000\n2\tA\t-8000000000\t28000000000\n3\tB\t-7000000000\t29000000000\n"
    );
    fs::remove_dir_all(db).unwrap();
}
