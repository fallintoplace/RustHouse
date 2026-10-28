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
fn parallel_grouped_distinct_unions_duplicates_across_physical_parts() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-grouped-distinct-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE events (region UInt8, user_id UInt64, flag UInt8)", &db);
    for index in 0..40u64 {
        let region = index % 4;
        let user = 5_000_000_000u64 + index % 10;
        sql(&format!(
            "INSERT INTO events VALUES ({region}, {user}, 1), ({region}, {user}, 0)"
        ), &db);
    }
    assert_eq!(
        sql("SELECT region, count(DISTINCT user_id) FROM events GROUP BY region ORDER BY region", &db),
        "region\tcountDistinct(user_id)\n0\t5\n1\t5\n2\t5\n3\t5\n"
    );
    assert_eq!(
        sql("SELECT region, count(DISTINCT user_id) FROM events WHERE flag = 1 GROUP BY region ORDER BY region", &db),
        "region\tcountDistinct(user_id)\n0\t5\n1\t5\n2\t5\n3\t5\n"
    );
    fs::remove_dir_all(db).unwrap();
}
