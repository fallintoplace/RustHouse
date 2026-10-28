use std::fs;
use std::process::Command;

#[test]
fn joined_multi_dimension_count_groups_fact_and_dimension_keys() {
    let db = std::env::temp_dir().join(format!("rusthouse-joined-multi-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE fact (id UInt64, region String)",
        "CREATE TABLE dim (id UInt64, segment String)",
        "INSERT INTO fact VALUES (1, 'EU'), (1, 'EU'), (2, 'US'), (3, 'EU')",
        "INSERT INTO dim VALUES (1, 'retail'), (2, 'retail'), (3, 'games')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT f.region, d.segment, count(*) FROM fact f JOIN dim d ON f.id = d.id GROUP BY f.region, d.segment",
            b"f.region\td.segment\tcount\nEU\tretail\t2\nUS\tretail\t1\nEU\tgames\t1\n".as_slice(),
        ),
        (
            "SELECT f.region, d.segment, count(*) FROM fact f JOIN dim d ON f.id = d.id GROUP BY f.region, d.segment HAVING count(*) > 1",
            b"f.region\td.segment\tcount\nEU\tretail\t2\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
