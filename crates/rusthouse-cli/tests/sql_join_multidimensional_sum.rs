use std::fs;
use std::process::Command;

#[test]
fn exact_composite_join_sum_preserves_negative_and_all_null_metric_groups() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-multi-sum-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, region String, revenue Nullable(Int64))",
        "CREATE TABLE dim (id UInt64, segment String)",
        "INSERT INTO facts VALUES (1, 'EU', 10), (1, 'EU', -2), (2, 'EU', NULL), (3, 'US', 4), (3, 'US', 6)",
        "INSERT INTO dim VALUES (1, 'retail'), (2, 'games'), (3, 'retail')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT f.region, d.segment, sum(f.revenue) FROM facts f JOIN dim d ON f.id = d.id GROUP BY f.region, d.segment",
            b"f.region\td.segment\tsum(f.revenue)\nEU\tretail\t8\nEU\tgames\tNULL\nUS\tretail\t10\n".as_slice(),
        ),
        (
            "SELECT f.region, d.segment, sum(f.revenue) FROM facts f JOIN dim d ON f.id = d.id GROUP BY f.region, d.segment HAVING sum(f.revenue) > 8 ORDER BY 3 DESC LIMIT 1",
            b"f.region\td.segment\tsum(f.revenue)\nUS\tretail\t10\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
