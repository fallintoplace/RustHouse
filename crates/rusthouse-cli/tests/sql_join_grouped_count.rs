use std::fs;
use std::process::Command;

#[test]
fn star_schema_grouped_count_streams_large_fact_side() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-group-count-{}", std::process::id()));
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
        (
            "SELECT d.id, count(*) FROM facts f JOIN dimensions d ON f.id = d.id GROUP BY d.id LIMIT 3",
            b"d.id\tcount\n0\t5\n1\t5\n2\t5\n".as_slice(),
        ),
        (
            "SELECT f.id, count(*) FROM facts f JOIN dimensions d ON f.id = d.id GROUP BY f.id LIMIT 2 OFFSET 2",
            b"f.id\tcount\n2\t5\n3\t5\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
