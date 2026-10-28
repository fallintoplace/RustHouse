use std::fs;
use std::process::Command;

#[test]
fn count_distinct_joined_fact_keys_is_exact_across_many_physical_parts() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-distinct-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dims (id UInt64)",
        "INSERT INTO dims SELECT number FROM numbers(8192)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for _ in 0..5 {
        let result = sql("INSERT INTO facts SELECT number FROM numbers(8192)");
        assert!(result.status.success(), "{result:?}");
    }
    let result = sql(
        "SELECT count(DISTINCT f.id) FROM facts f JOIN dims d ON f.id = d.id"
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"count(DISTINCT f.id)\n8192\n");
    fs::remove_dir_all(db).unwrap();
}
