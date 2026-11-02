use std::fs;
use std::process::Command;

#[test]
fn taxi_three_key_group_having_and_rank_matches_reference() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-taxi-route-rank-{}",
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
        "CREATE TABLE trips (pickup_day UInt32, pickup_zone UInt32, dropoff_zone UInt32, fare_cents UInt64, distance_milli UInt64)",
        "INSERT INTO trips VALUES (20240101, 1, 5, 6000, 12000), (20240101, 1, 5, 6000, 12000), (20240101, 1, 5, 6000, 12000), (20240101, 1, 6, 5000, 15000), (20240101, 1, 6, 5000, 15000), (20240101, 1, 6, 5000, 15000), (20240101, 1, 6, 5000, 15000), (20240102, 2, 8, 7000, 18000), (20240102, 2, 8, 7000, 18000), (20240102, 2, 8, 7000, 18000), (20240102, 2, 8, 7000, 18000), (20240102, 2, 8, 7000, 18000), (20240103, 2, 8, 9000, 30000), (20240103, 2, 8, 9000, 30000), (20240103, 2, 8, 9000, 30000), (20240104, 2, 9, 7000, 11000), (20240104, 2, 9, 7000, 11000), (20240101, 1, 5, 4000, 12000), (20240101, 1, 6, 5000, 9000)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }

    let query = "SELECT pickup_day, pickup_zone, dropoff_zone, COUNT(*) AS rides FROM trips WHERE fare_cents >= 5000 AND distance_milli >= 10000 GROUP BY pickup_day, pickup_zone, dropoff_zone HAVING COUNT(*) >= 3 ORDER BY rides DESC, pickup_day, pickup_zone, dropoff_zone LIMIT 20";
    let output = sql(query);
    assert!(output.status.success(), "{query}: {output:?}");
    assert_eq!(
        output.stdout,
        b"pickup_day\tpickup_zone\tdropoff_zone\trides\n20240102\t2\t8\t5\n20240101\t1\t6\t4\n20240101\t1\t5\t3\n20240103\t2\t8\t3\n"
    );

    // HAVING eliminates all groups, but preserves the output schema.
    let empty = sql("SELECT pickup_day, pickup_zone, dropoff_zone, COUNT(*) AS rides FROM trips WHERE fare_cents >= 99999 GROUP BY pickup_day, pickup_zone, dropoff_zone HAVING COUNT(*) >= 3 ORDER BY rides DESC, pickup_day, pickup_zone, dropoff_zone LIMIT 20");
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"pickup_day\tpickup_zone\tdropoff_zone\trides\n");

    fs::remove_dir_all(db).unwrap();
}
