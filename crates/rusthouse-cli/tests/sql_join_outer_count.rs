use std::fs;
use std::process::Command;

#[test]
fn exact_outer_join_counts_preserve_duplicate_and_null_extended_rows() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-outer-join-count-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lefts (id UInt64, flag Nullable(UInt64))",
        "CREATE TABLE rights (id UInt64, flag Nullable(UInt64))",
        "INSERT INTO lefts VALUES (1, 1), (1, NULL), (2, 2)",
        "INSERT INTO rights VALUES (1, 1), (1, NULL), (3, 3)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        ("SELECT COUNT(*) FROM lefts l LEFT JOIN rights r ON l.id = r.id", b"count\n5\n".as_slice()),
        ("SELECT COUNT(*) FROM lefts l RIGHT JOIN rights r ON l.id = r.id", b"count\n5\n"),
        ("SELECT COUNT(*) FROM lefts l FULL JOIN rights r ON l.id = r.id", b"count\n6\n"),
        ("SELECT COUNT(*) FROM lefts l LEFT JOIN rights r ON l.id = r.id WHERE r.flag IS NULL", b"count\n3\n"),
        ("SELECT COUNT(*) FROM lefts l RIGHT JOIN rights r ON l.id = r.id WHERE l.flag IS NULL", b"count\n3\n"),
        ("SELECT COUNT(*) FROM lefts l LEFT JOIN rights r ON l.id = r.id LIMIT 0", b"count\n"),
        ("SELECT COUNT(*) FROM lefts l FULL JOIN rights r ON l.id = r.id OFFSET 1", b"count\n"),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
