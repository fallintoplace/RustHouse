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
fn parallel_distinct_bitmaps_merge_exactly_across_many_parts() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-exact-parallel-distinct-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (category UInt16, signed_category Int8, flag UInt8)", &db);
    for index in 0..40u64 {
        let category = 60000 + index % 13;
        let signed = index as i64 % 7 - 3;
        sql(&format!(
            "INSERT INTO facts VALUES ({category}, {signed}, 1), ({category}, {signed}, 0)"
        ), &db);
    }
    assert_eq!(
        sql("SELECT count(DISTINCT category) FROM facts", &db),
        "countDistinct(category)\n13\n"
    );
    assert_eq!(
        sql("SELECT count(DISTINCT signed_category) FROM facts", &db),
        "countDistinct(signed_category)\n7\n"
    );
    assert_eq!(
        sql("SELECT count(DISTINCT category) FROM facts WHERE flag = 1", &db),
        "countDistinct(category)\n13\n"
    );
    assert_eq!(
        sql("SELECT count(DISTINCT category) FROM facts WHERE flag = 2", &db),
        "countDistinct(category)\n0\n"
    );
    fs::remove_dir_all(db).unwrap();
}
