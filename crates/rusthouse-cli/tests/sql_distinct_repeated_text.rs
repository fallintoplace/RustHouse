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
fn distinct_deduplicates_repeated_strings_across_parts_before_pagination() {
    let db = std::env::temp_dir().join(format!("rusthouse-distinct-text-{}", std::process::id()));
    for query in [
        "CREATE TABLE events (region String, amount UInt64)",
        "INSERT INTO events VALUES ('EU', 1), ('EU', 1), ('US', 2)",
        "INSERT INTO events VALUES ('EU', 1), ('EU', 3), ('US', 2)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    let result = sql(
        "SELECT DISTINCT region, amount FROM events ORDER BY region, amount",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"region\tamount\nEU\t1\nEU\t3\nUS\t2\n");

    let page = sql(
        "SELECT DISTINCT region, amount FROM events ORDER BY region, amount LIMIT 1 OFFSET 1",
        &db,
    );
    assert!(page.status.success(), "{page:?}");
    assert_eq!(page.stdout, b"region\tamount\nEU\t3\n");
    fs::remove_dir_all(db).unwrap();
}
