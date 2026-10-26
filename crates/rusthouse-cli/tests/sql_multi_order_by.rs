use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(db: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

fn success(db: &Path, statement: &str) -> String {
    let result = sql(db, statement);
    assert!(result.status.success(), "{statement}: {result:?}");
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn orders_stored_numeric_and_typed_rows_by_multiple_columns() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-multi-order-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    success(
        &db,
        "CREATE TABLE events (category String, priority UInt64, seq UInt64)",
    );
    success(
        &db,
        "INSERT INTO events VALUES ('b',1,1),('a',2,2),('a',1,3),('b',1,4),('c',2,5)",
    );

    assert_eq!(
        success(
            &db,
            "SELECT category, priority, seq FROM events ORDER BY category ASC, priority DESC, seq ASC",
        ),
        "category\tpriority\tseq\na\t2\t2\na\t1\t3\nb\t1\t1\nb\t1\t4\nc\t2\t5\n"
    );
    assert_eq!(
        success(
            &db,
            "SELECT priority, seq FROM events ORDER BY priority ASC, seq DESC LIMIT 2 OFFSET 1",
        ),
        "priority\tseq\n1\t3\n1\t1\n"
    );
    assert_eq!(
        success(
            &db,
            "SELECT category, count(*) FROM events GROUP BY category ORDER BY count DESC, category DESC",
        ),
        "category\tcount\nb\t2\na\t2\nc\t1\n"
    );
    assert_eq!(
        success(
            &db,
            "SELECT priority FROM events WHERE priority > 0 ORDER BY priority ASC, seq DESC LIMIT 1",
        ),
        "priority\n1\n"
    );

    let distinct = sql(
        &db,
        "SELECT DISTINCT category FROM events ORDER BY category, priority",
    );
    assert!(!distinct.status.success(), "{distinct:?}");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn uses_secondary_sort_keys_for_grouped_numbers() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT number, count(*) FROM numbers(3) GROUP BY number ORDER BY count DESC, number DESC",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"number\tcount\n2\t1\n1\t1\n0\t1\n");
}
