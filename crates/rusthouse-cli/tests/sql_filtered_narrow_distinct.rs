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
fn filtered_narrow_distinct_retains_exact_bitmap_semantics() {
    let db = std::env::temp_dir().join(format!("rusthouse-filtered-bitmap-{}", std::process::id()));
    for query in [
        "CREATE TABLE events (signed_key Int16, key UInt16, flag UInt8)",
        "INSERT INTO events VALUES (-32768, 65535, 1), (-1, 4, 0), (-1, 4, 1)",
        "INSERT INTO events VALUES (32767, 0, 1), (-32768, 65535, 0), (32767, 0, 1)",
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT count(DISTINCT signed_key) FROM events WHERE flag = 1",
            3,
        ),
        ("SELECT count(DISTINCT key) FROM events WHERE flag = 0", 2),
        ("SELECT count(DISTINCT key) FROM events WHERE flag = 2", 0),
        ("SELECT count(DISTINCT key) FROM events", 3),
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
        let actual = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            actual.lines().last().unwrap().parse::<u64>().unwrap(),
            expected,
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
