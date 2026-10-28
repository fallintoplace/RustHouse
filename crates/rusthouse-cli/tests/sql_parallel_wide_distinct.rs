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
fn parallel_wide_count_distinct_unions_cross_part_user_ids() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-wide-distinct-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE events (user_id UInt64, flag UInt8)", &db);
    for index in 0..40u64 {
        let user = 5_000_000_000 + index % 17;
        sql(&format!(
            "INSERT INTO events VALUES ({user}, 1), ({user}, 0)"
        ), &db);
    }
    assert_eq!(
        sql("SELECT count(DISTINCT user_id) FROM events", &db),
        "countDistinct(user_id)\n17\n"
    );
    assert_eq!(
        sql("SELECT count(DISTINCT user_id) FROM events WHERE flag = 1", &db),
        "countDistinct(user_id)\n17\n"
    );
    assert_eq!(
        sql("SELECT count(DISTINCT user_id) FROM events WHERE flag = 2", &db),
        "countDistinct(user_id)\n0\n"
    );
    fs::remove_dir_all(db).unwrap();
}
