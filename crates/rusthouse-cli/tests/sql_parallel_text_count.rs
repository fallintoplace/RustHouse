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
fn parallel_text_filtered_count_reduces_uneven_parts() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-text-count-{}",
        std::process::id()
    ));
    for statement in ["CREATE TABLE events (id UInt64, category String)"] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for n in 0..36 {
        let statement = format!(
            "INSERT INTO events VALUES ({n}, 'yes'), ({}, 'no')",
            n + 100
        );
        let result = sql(&statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, count) in [
        ("SELECT count(*) FROM events WHERE category = 'yes'", 36),
        ("SELECT count(*) FROM events WHERE category = 'no'", 36),
        ("SELECT count(*) FROM events WHERE category = 'other'", 0),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            format!("count\n{count}\n"),
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
