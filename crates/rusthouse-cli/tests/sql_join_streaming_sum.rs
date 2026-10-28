use std::fs;
use std::process::Command;

#[test]
fn streaming_join_sum_aggregates_large_exact_fact_values() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-sum-large-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimensions (id UInt64)",
        "INSERT INTO dimensions SELECT number FROM numbers(8192)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for _ in 0..5 {
        let result = sql("INSERT INTO facts SELECT number FROM numbers(8192)");
        assert!(result.status.success(), "{result:?}");
    }
    for (query, expected) in [
        ("SELECT SUM(f.id) FROM facts f JOIN dimensions d ON f.id = d.id", b"sum(f.id)\n167751680\n".as_slice()),
        ("SELECT SUM(d.id) FROM facts f JOIN dimensions d ON f.id = d.id", b"sum(d.id)\n167751680\n"),
        ("SELECT SUM(f.id) FROM facts f JOIN dimensions d ON f.id = d.id LIMIT 0", b"sum(f.id)\n"),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
