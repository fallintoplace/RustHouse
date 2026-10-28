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
fn parallel_group_sum_handles_signed_integer_values_across_many_parts() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-group-sum-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (group_id UInt16, amount Int64, flag UInt8)", &db);
    for index in 0..40 {
        let group = index % 4;
        sql(&format!(
            "INSERT INTO facts VALUES ({group}, -5000000000, 1), ({group}, 6000000000, 0)"
        ), &db);
    }
    assert_eq!(
        sql("SELECT group_id, sum(amount) FROM facts GROUP BY group_id ORDER BY group_id", &db),
        "group_id\tsum(amount)\n0\t10000000000\n1\t10000000000\n2\t10000000000\n3\t10000000000\n"
    );
    assert_eq!(
        sql("SELECT group_id, sum(amount) FROM facts WHERE flag = 1 GROUP BY group_id ORDER BY group_id", &db),
        "group_id\tsum(amount)\n0\t-50000000000\n1\t-50000000000\n2\t-50000000000\n3\t-50000000000\n"
    );
    fs::remove_dir_all(db).unwrap();
}
