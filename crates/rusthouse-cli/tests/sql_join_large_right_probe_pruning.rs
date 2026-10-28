use std::fs;
use std::process::Command;

#[test]
fn sparse_join_on_large_nullable_right_table_preserves_exact_result() {
    let db = std::env::temp_dir().join(format!("rusthouse-right-probe-{}",std::process::id()));
    let sql = |query: &str| Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql",query,"--database"]).arg(&db).output().unwrap();
    for query in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE events (id UInt64, payload Nullable(String))",
        "INSERT INTO facts VALUES (45000)",
        "INSERT INTO events (id) SELECT number FROM numbers(50000)",
    ] {
        let result = sql(query);
        assert!(result.status.success(),"{query}: {result:?}");
    }
    let result = sql("SELECT f.id, e.id FROM facts f JOIN events e ON f.id = e.id LIMIT 1");
    assert!(result.status.success(),"{result:?}");
    assert_eq!(result.stdout,b"f.id\te.id\n45000\t45000\n");
    fs::remove_dir_all(db).unwrap();
}
