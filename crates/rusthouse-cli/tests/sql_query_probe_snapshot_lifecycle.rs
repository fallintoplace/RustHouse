use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-query-lifecycle-probe-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn run(db: &Path, statement: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn snapshot_rewrites_keep_queries_consistent_before_and_after_changes() {
    let db = database();
    for statement in [
        "CREATE TABLE accounts (id UInt64, bucket String, amount Nullable(Int64))",
        "INSERT INTO accounts VALUES (1, 'a', 10), (2, 'a', 20), (3, 'b', NULL), (4, 'b', 30), (5, 'c', 0)",
    ] {
        run(&db, statement);
    }
    for (sql, expected) in [
        (
            "SELECT id, amount FROM accounts ORDER BY id DESC LIMIT 3 OFFSET 1",
            "id\tamount\n4\t30\n3\t\\N\n2\t20\n",
        ),
        (
            "SELECT bucket, sum(amount) FROM accounts GROUP BY bucket ORDER BY bucket",
            "bucket\tsum(amount)\na\t30\nb\t30\nc\t0\n",
        ),
    ] {
        assert_eq!(run(&db, sql), expected, "{sql}");
    }

    run(&db, "UPDATE accounts SET amount = 15 WHERE id = 1");
    run(&db, "DELETE FROM accounts WHERE id = 4");

    for (sql, expected) in [
        (
            "SELECT bucket, count(*) AS n FROM accounts GROUP BY bucket ORDER BY bucket DESC",
            "bucket\tn\nc\t1\nb\t1\na\t2\n",
        ),
        (
            "SELECT id FROM accounts WHERE amount IS NULL OR amount >= 15 ORDER BY id",
            "id\n1\n2\n3\n",
        ),
    ] {
        assert_eq!(run(&db, sql), expected, "{sql}");
    }
    fs::remove_dir_all(db).unwrap();
}
