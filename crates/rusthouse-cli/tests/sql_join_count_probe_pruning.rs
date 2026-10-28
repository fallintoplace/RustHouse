use std::fs;
use std::process::Command;

#[test]
fn large_fact_count_decodes_only_join_key_not_nullable_payload() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-count-probe-{}",std::process::id()));
    let sql = |query: &str| Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql",query,"--database"]).arg(&db).output().unwrap();
    for query in [
        "CREATE TABLE facts (id UInt64, note Nullable(String))",
        "CREATE TABLE dimensions (id UInt64)",
        "INSERT INTO facts (id) SELECT number FROM numbers(45000)",
        "INSERT INTO dimensions SELECT number FROM numbers(8192)",
    ] {
        let result = sql(query);
        assert!(result.status.success(),"{query}: {result:?}");
    }
    let result = sql("SELECT count(*) FROM facts f INNER JOIN dimensions d ON f.id = d.id");
    assert!(result.status.success(),"{result:?}");
    assert_eq!(result.stdout, b"count\n8192\n");
    fs::remove_dir_all(db).unwrap();
}
