use std::fs;
use std::process::Command;

#[test]
fn joined_revenue_and_event_count_are_fused_over_fact_stream() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-fused-metrics-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, region String, amount Nullable(Int64))",
        "CREATE TABLE dim (id UInt64, segment String)",
        "INSERT INTO facts VALUES (1, 'EU', 10), (1, 'EU', -2), (2, 'EU', NULL), (3, 'US', 6)",
        "INSERT INTO dim VALUES (1, 'retail'), (2, 'games'), (3, 'retail')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT f.region, d.segment, sum(f.amount), count(*) FROM facts f JOIN dim d ON f.id = d.id GROUP BY f.region, d.segment";
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"f.region\td.segment\tsum(f.amount)\tcount\nEU\tretail\t8\t2\nEU\tgames\tNULL\t1\nUS\tretail\t6\t1\n"
    );
    fs::remove_dir_all(db).unwrap();
}
