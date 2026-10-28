use std::fs;
use std::process::Command;

#[test]
fn streaming_right_join_preserves_unmatched_rows_with_pruned_left_payloads() {
    let db = std::env::temp_dir().join(format!("rusthouse-right-pruned-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"]).arg(&db).output().unwrap()
    };
    for query in [
        "CREATE TABLE lookup (id UInt64, payload String)",
        "CREATE TABLE events (id UInt64)",
        "INSERT INTO lookup VALUES (1, 'unused'), (2, 'unused')",
        "INSERT INTO events SELECT number FROM numbers(40000)",
    ] {
        let out = sql(query);
        assert!(out.status.success(), "{query}: {out:?}");
    }
    let out = sql("SELECT e.id FROM lookup l RIGHT JOIN events e ON l.id = e.id LIMIT 3");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"e.id\n0\n1\n2\n");
    fs::remove_dir_all(db).unwrap();
}
