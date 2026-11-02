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
fn filters_narrow_numeric_groups_across_many_parts() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-filtered-group-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (bucket Int32, flag UInt8)", &db);
    for i in 0..40 {
        sql(&format!(
            "INSERT INTO facts VALUES ({}, 1), ({}, 0), ({}, 1)",
            i % 4 - 2, i % 4 - 2, i % 4 - 2
        ), &db);
    }
    assert_eq!(
        sql("SELECT bucket, count(*) FROM facts WHERE flag = 1 GROUP BY bucket ORDER BY bucket", &db),
        "bucket\tcount\n-2\t20\n-1\t20\n0\t20\n1\t20\n"
    );
    assert_eq!(
        sql("SELECT bucket, count(*) FROM facts WHERE flag = 0 GROUP BY bucket ORDER BY bucket", &db),
        "bucket\tcount\n-2\t10\n-1\t10\n0\t10\n1\t10\n"
    );
    fs::remove_dir_all(db).unwrap();
}
