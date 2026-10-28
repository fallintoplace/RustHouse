use std::fs;
use std::process::Command;

#[test]
fn byte_distinct_handles_signed_unsigned_nulls_and_filters() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-distinct-byte-bitmap-{}",
        std::process::id()
    ));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (signed_key Nullable(Int8), positive_key Nullable(UInt8), flag UInt8)",
        "INSERT INTO events VALUES (-128, 255, 1), (-128, 255, 1), (127, 0, 0), (NULL, NULL, 1)",
        "INSERT INTO events VALUES (-1, 42, 1), (127, 0, 1), (NULL, 255, 1)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        ("SELECT count(DISTINCT signed_key) FROM events", "countDistinct(signed_key)\n3\n"),
        ("SELECT count(DISTINCT positive_key) FROM events", "countDistinct(positive_key)\n3\n"),
        ("SELECT count(DISTINCT signed_key) FROM events WHERE flag = 1", "countDistinct(signed_key)\n3\n"),
        ("SELECT count(DISTINCT positive_key) FROM events WHERE flag = 0", "countDistinct(positive_key)\n1\n"),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
