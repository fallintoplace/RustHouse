use std::fs;
use std::process::Command;

#[test]
fn join_count_does_not_need_wide_nullable_build_payloads() {
    let db = std::env::temp_dir().join(format!("rusthouse-count-pruned-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"]).arg(&db).output().unwrap()
    };
    for stmt in [
        "CREATE TABLE fact (id UInt64)",
        "CREATE TABLE dim (id UInt64, payload Nullable(String))",
        "INSERT INTO dim (id) SELECT number FROM numbers(8192)",
        "INSERT INTO fact SELECT number FROM numbers(45000)",
    ] {
        let output = sql(stmt);
        assert!(output.status.success(), "{stmt}: {output:?}");
    }
    let output = sql("SELECT count(*) FROM fact f JOIN dim d ON f.id = d.id");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"count\n8192\n");
    fs::remove_dir_all(db).unwrap();
}
