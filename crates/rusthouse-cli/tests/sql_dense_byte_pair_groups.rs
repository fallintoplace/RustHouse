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
fn groups_two_byte_dimensions_with_and_without_filters() {
    let db = std::env::temp_dir().join(format!("rusthouse-byte-pair-{}", std::process::id()));
    for query in [
        "CREATE TABLE events (u UInt8, s Int8, flag UInt8)",
        "INSERT INTO events VALUES (255, -128, 1), (0, 127, 0), (1, -1, 1)",
        "INSERT INTO events VALUES (255, -128, 1), (0, 127, 1), (1, -1, 1)",
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT u, s, count(*) FROM events GROUP BY u, s ORDER BY u, s",
            "u\ts\tcount\n0\t127\t2\n1\t-1\t2\n255\t-128\t2\n",
        ),
        (
            "SELECT u, s, count(*) FROM events WHERE flag = 1 GROUP BY u, s ORDER BY u, s",
            "u\ts\tcount\n0\t127\t1\n1\t-1\t2\n255\t-128\t2\n",
        ),
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected,
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
