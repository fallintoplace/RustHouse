use std::fs;
use std::process::Command;

#[test]
fn taxi_fare_tip_comparison_feeds_five_aggregate_metrics() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-taxi-five-metrics-{}",
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
        "CREATE TABLE trips (fare_cents UInt64, tip_cents UInt64)",
        "INSERT INTO trips VALUES (1000,100), (2000,200), (300,300), (0,0), (900,1000), (100,100)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT SUM(fare_cents) AS total_fare, AVG(tip_cents) AS average_tip, MIN(fare_cents) AS lowest_fare, MAX(tip_cents) AS highest_tip, COUNT(*) AS rides FROM trips WHERE fare_cents > tip_cents";
    let output = sql(query);
    assert!(output.status.success(), "{query}: {output:?}");
    assert_eq!(
        output.stdout,
        b"total_fare\taverage_tip\tlowest_fare\thighest_tip\trides\n3000\t150\t1000\t200\t2\n"
    );

    // Equality is deliberately included only by >=, not by >.
    let equal_or_greater = sql("SELECT COUNT(*) AS matching FROM trips WHERE fare_cents >= tip_cents");
    assert!(equal_or_greater.status.success(), "{equal_or_greater:?}");
    assert_eq!(equal_or_greater.stdout, b"matching\n5\n");
    fs::remove_dir_all(db).unwrap();
}
