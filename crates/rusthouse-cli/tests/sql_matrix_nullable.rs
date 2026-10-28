use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-query-matrix-nullable-{}-{}",
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
fn nullable_filters_and_aggregates_match_reference_results() {
    let db = database();
    for statement in [
        "CREATE TABLE events (id UInt64, region String, amount Nullable(UInt64), note Nullable(String))",
        "INSERT INTO events VALUES (1, 'north', 10, 'ok'), (2, 'south', NULL, NULL), (3, 'north', 20, NULL), (4, 'east', 5, 'x'), (5, 'south', 0, 'ok'), (6, 'east', NULL, 'ok'), (7, 'north', 10, 'z'), (8, 'south', 30, NULL)",
    ] {
        run(&db, statement);
    }
    for (statement, expected) in [
        (
            "SELECT region, count(*) AS n FROM events WHERE amount IS NOT NULL GROUP BY region ORDER BY region",
            "region\tn\neast\t1\nnorth\t3\nsouth\t2\n",
        ),
        (
            "SELECT region, sum(amount), count(amount) FROM events GROUP BY region ORDER BY region",
            "region\tsum(amount)\tcount(amount)\neast\t5\t1\nnorth\t40\t3\nsouth\t30\t2\n",
        ),
        (
            "SELECT id, region FROM events WHERE note IS NULL OR amount = 0 ORDER BY id DESC LIMIT 2 OFFSET 1",
            "id\tregion\n5\tsouth\n3\tnorth\n",
        ),
        (
            "SELECT count(DISTINCT region), count(note) FROM events WHERE amount >= 10",
            "countDistinct(region)\tcount(note)\n2\t2\n",
        ),
    ] {
        assert_eq!(run(&db, statement), expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
