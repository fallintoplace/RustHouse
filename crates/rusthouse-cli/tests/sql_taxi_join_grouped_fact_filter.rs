use std::fs;
use std::process::Command;

#[test]
fn grouped_inner_join_filters_fact_rows_before_counting_matches() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-taxi-join-group-filter-{}",
        std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE trips (pickup_zone UInt64, fare_cents UInt64)",
        "CREATE TABLE zones (zone_id UInt64, borough String)",
        "INSERT INTO trips VALUES (1, 1500), (1, 5000), (2, 2100), (2, 9000), (3, 3000), (4, 10000)",
        "INSERT INTO zones VALUES (1, 'Manhattan'), (2, 'Brooklyn'), (3, 'Brooklyn')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT z.borough, COUNT(*) AS n FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id WHERE t.fare_cents > 2000 GROUP BY z.borough ORDER BY z.borough";
    let result = sql(query);
    assert!(result.status.success(), "{query}: {result:?}");
    assert_eq!(result.stdout, b"z.borough\tn\nBrooklyn\t3\nManhattan\t1\n");
    let query = "SELECT z.borough, COUNT(*) FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id WHERE t.fare_cents < 2000 GROUP BY z.borough ORDER BY z.borough";
    let result = sql(query);
    assert!(result.status.success(), "{query}: {result:?}");
    assert_eq!(result.stdout, b"z.borough\tcount\nManhattan\t1\n");
    let unsupported = "SELECT z.borough, COUNT(*) FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id WHERE z.zone_id > 2 GROUP BY z.borough";
    assert!(!sql(unsupported).status.success());
    fs::remove_dir_all(db).unwrap();
}
