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
fn grouped_uuid_distinct_counts_duplicates_across_parts() {
    let db = std::env::temp_dir().join(format!("rusthouse-group-uuid-{}", std::process::id()));
    for query in [
        "CREATE TABLE hits (category String, user_id UUID)",
        "INSERT INTO hits VALUES ('A', '00000000-0000-0000-0000-000000000001'), ('A', '00000000-0000-0000-0000-000000000002'), ('B', '00000000-0000-0000-0000-000000000001')",
        "INSERT INTO hits VALUES ('A', '00000000-0000-0000-0000-000000000001'), ('B', '00000000-0000-0000-0000-000000000003')",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    let result = sql(
        "SELECT category, count(DISTINCT user_id) FROM hits GROUP BY category ORDER BY category",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"category\tcountDistinct(user_id)\nA\t2\nB\t2\n"
    );
    fs::remove_dir_all(db).unwrap();
}
