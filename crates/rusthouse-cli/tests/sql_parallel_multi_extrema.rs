use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, database: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap();
    assert!(output.status.success(), "{statement}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn parallel_multi_aggregates_merge_signed_integer_extrema() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-multi-extrema-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql(
        "CREATE TABLE facts (amount Int64, status UInt8)",
        &root,
    );
    for i in 0..40 {
        sql(
            &format!(
                "INSERT INTO facts VALUES ({}, 1), ({}, 0)",
                i - 20,
                1_000 - i
            ),
            &root,
        );
    }
    let result = sql(
        "SELECT min(amount), max(amount), sum(amount), count(*) FROM facts WHERE status = 1",
        &root,
    );
    assert_eq!(result.lines().nth(1), Some("-20\t19\t-20\t40"));
    fs::remove_dir_all(root).unwrap();
}
