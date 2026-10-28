use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, db: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(output.status.success(), "{statement}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn parallel_dense_groups_reduce_all_immutable_parts() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-grouped-count-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (category UInt16, signed_category Int8)", &db);
    for index in 0..40 {
        sql(&format!(
            "INSERT INTO facts VALUES ({}, {}), ({}, {})",
            index % 4, index % 4 - 2, index % 4, index % 4 - 2
        ), &db);
    }
    assert_eq!(
        sql("SELECT category, count(*) FROM facts GROUP BY category ORDER BY category", &db),
        "category\tcount\n0\t20\n1\t20\n2\t20\n3\t20\n"
    );
    assert_eq!(
        sql("SELECT signed_category, count(*) FROM facts GROUP BY signed_category ORDER BY signed_category", &db),
        "signed_category\tcount\n-2\t20\n-1\t20\n0\t20\n1\t20\n"
    );
    assert_eq!(
        sql("SELECT category, count(*) FROM facts GROUP BY category ORDER BY count DESC LIMIT 2", &db).lines().count(),
        3
    );
    fs::remove_dir_all(db).unwrap();
}
