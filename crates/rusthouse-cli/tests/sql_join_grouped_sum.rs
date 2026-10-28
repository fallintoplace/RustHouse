use std::fs;
use std::process::Command;

#[test]
fn grouped_join_sum_streams_facts_and_preserves_exact_null_semantics() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-group-sum-{}", std::process::id()));
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
    let result = sql(
        "SELECT d.id, sum(f.id) FROM facts f JOIN dimensions d ON f.id = d.id GROUP BY d.id LIMIT 3"
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"d.id\tsum(f.id)\n0\t0\n1\t5\n2\t10\n");
    fs::remove_dir_all(&db).unwrap();

    let nullable = std::env::temp_dir().join(format!("rusthouse-join-group-sum-null-{}", std::process::id()));
    let sql_null = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&nullable)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (id UInt64, revenue Nullable(Int64))",
        "CREATE TABLE labels (id UInt64)",
        "INSERT INTO labels VALUES (1), (2), (3)",
        "INSERT INTO events VALUES (1, 10), (2, NULL), (3, -2), (1, -2)",
    ] {
        let result = sql_null(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql_null(
        "SELECT d.id, sum(f.revenue) FROM events f JOIN labels d ON f.id = d.id GROUP BY d.id"
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"d.id\tsum(f.revenue)\n1\t8\n2\tNULL\n3\t-2\n");
    fs::remove_dir_all(nullable).unwrap();
}
