use std::fs;
use std::process::Command;

#[test]
fn large_right_outer_probe_keeps_unmatched_row_count_with_nullable_payloads() {
    let db = std::env::temp_dir().join(format!("rusthouse-right-probe-outer-{}",std::process::id()));
    let sql = |query: &str| Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql",query,"--database"]).arg(&db).output().unwrap();
    for query in [
        "CREATE TABLE lookup (id UInt64)",
        "CREATE TABLE events (id UInt64, payload Nullable(String))",
        "INSERT INTO lookup VALUES (2)",
        "INSERT INTO events (id) SELECT number FROM numbers(45000)",
    ] {
        let result = sql(query);
        assert!(result.status.success(),"{query}: {result:?}");
    }
    let result = sql("SELECT l.id, e.id FROM lookup l RIGHT JOIN events e ON l.id = e.id LIMIT 3");
    assert!(result.status.success(),"{result:?}");
    assert_eq!(result.stdout,b"l.id\te.id\n\\N\t0\n\\N\t1\n2\t2\n");
    fs::remove_dir_all(db).unwrap();
}
