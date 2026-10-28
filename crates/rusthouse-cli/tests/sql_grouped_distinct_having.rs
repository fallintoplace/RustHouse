use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(query: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn having_filters_grouped_distinct_counts_after_null_elimination() {
    let db = std::env::temp_dir().join(format!("rusthouse-having-distinct-{}", std::process::id()));
    for query in [
        "CREATE TABLE events (category String, user_id Nullable(String))",
        "INSERT INTO events VALUES ('A', 'u1'), ('A', 'u1'), ('A', 'u2'), ('B', 'u1'), ('B', NULL), ('C', NULL)",
        "INSERT INTO events VALUES ('A', NULL), ('B', 'u1'), ('C', NULL)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    let filtered = sql(
        "SELECT category, count(DISTINCT user_id) FROM events GROUP BY category HAVING count(DISTINCT user_id) > 1 ORDER BY category",
        &db,
    );
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(filtered.stdout, b"category\tcountDistinct(user_id)\nA\t2\n");

    let positive = sql(
        "SELECT category, count(DISTINCT user_id) FROM events GROUP BY category HAVING count(DISTINCT user_id) >= 1 ORDER BY category",
        &db,
    );
    assert!(positive.status.success(), "{positive:?}");
    assert_eq!(positive.stdout, b"category\tcountDistinct(user_id)\nA\t2\nB\t1\n");

    let mismatch = sql(
        "SELECT category, count(DISTINCT user_id) FROM events GROUP BY category HAVING count(DISTINCT other) > 0",
        &db,
    );
    assert!(!mismatch.status.success());
    fs::remove_dir_all(db).unwrap();
}
