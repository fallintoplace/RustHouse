use std::fs;
use std::process::Command;

#[test]
fn composite_joined_exact_distinct_deduplicates_across_parts_and_ignores_nulls() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-group-distinct-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, region String, visitor Nullable(String))",
        "CREATE TABLE dim (id UInt64, segment String)",
        "INSERT INTO facts VALUES (1, 'EU', 'x'), (1, 'EU', 'x'), (2, 'EU', NULL)",
        "INSERT INTO facts VALUES (1, 'EU', 'y'), (3, 'US', 'x')",
        "INSERT INTO dim VALUES (1, 'retail'), (2, 'games'), (3, 'retail')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT f.region, d.segment, count(DISTINCT f.visitor) FROM facts f JOIN dim d ON f.id = d.id GROUP BY f.region, d.segment",
            b"f.region\td.segment\tcount(DISTINCT f.visitor)\nEU\tretail\t2\nEU\tgames\t0\nUS\tretail\t1\n".as_slice(),
        ),
        (
            "SELECT f.region, d.segment, count(DISTINCT f.visitor) FROM facts f JOIN dim d ON f.id = d.id GROUP BY f.region, d.segment HAVING count(DISTINCT f.visitor) > 0 ORDER BY 3 DESC LIMIT 2",
            b"f.region\td.segment\tcount(DISTINCT f.visitor)\nEU\tretail\t2\nUS\tretail\t1\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
