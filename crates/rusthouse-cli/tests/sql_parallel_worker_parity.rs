use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, database: &Path, threads: usize) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .env("RUSTHOUSE_QUERY_THREADS", threads.to_string())
        .output()
        .unwrap();
    assert!(output.status.success(), "{statement}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn parallel_query_results_match_serial_execution() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-parity-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql(
        "CREATE TABLE facts (category UInt8, amount Int64, status UInt8)",
        &root,
        1,
    );
    for part in 0..40 {
        sql(
            &format!(
                "INSERT INTO facts VALUES ({}, -12, 1), ({}, 30, 0), ({}, 100, 1)",
                part % 4,
                (part + 1) % 4,
                (part + 2) % 4
            ),
            &root,
            1,
        );
    }
    for query in [
        "SELECT count(*) FROM facts WHERE status = 1",
        "SELECT sum(amount) FROM facts WHERE status = 1",
        "SELECT min(amount) FROM facts WHERE status = 1",
        "SELECT max(amount) FROM facts WHERE status = 1",
        "SELECT category, count(*) FROM facts GROUP BY category ORDER BY category",
        "SELECT category, sum(amount) FROM facts GROUP BY category ORDER BY category",
        "SELECT count(DISTINCT category) FROM facts",
        "SELECT count(*), sum(amount) FROM facts",
        "SELECT category, count(*) FROM facts WHERE status = 1 GROUP BY category ORDER BY category",
    ] {
        let serial = sql(query, &root, 1);
        let parallel = sql(query, &root, 4);
        assert_eq!(parallel, serial, "query changed with thread count: {query}");
    }
    fs::remove_dir_all(root).unwrap();
}
