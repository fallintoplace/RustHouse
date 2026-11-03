use std::fs;
use std::process::Command;

#[test]
fn grouped_sum_case_matches_taxi_payment_revenue() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-taxi-grouped-case-{}",
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
        "CREATE TABLE trips (passenger_count UInt8, payment_type UInt8, fare_cents UInt64)",
        "INSERT INTO trips VALUES (1, 1, 1000), (1, 2, 2000), (1, 1, 300), (2, 2, 5000), (2, 1, 800)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT passenger_count, SUM(CASE WHEN payment_type = 1 THEN fare_cents ELSE 0 END) AS revenue FROM trips GROUP BY passenger_count ORDER BY passenger_count";
    let result = sql(query);
    assert!(result.status.success(), "{query}: {result:?}");
    assert_eq!(result.stdout, b"passenger_count\trevenue\n1\t1300\n2\t800\n");

    let query = "SELECT passenger_count, payment_type, SUM(CASE WHEN payment_type = 1 THEN fare_cents END) AS revenue FROM trips GROUP BY passenger_count, payment_type ORDER BY passenger_count, payment_type";
    let result = sql(query);
    assert!(result.status.success(), "{query}: {result:?}");
    assert_eq!(result.stdout, b"passenger_count\tpayment_type\trevenue\n1\t1\t1300\n1\t2\t0\n2\t1\t800\n2\t2\t0\n");

    for invalid in [
        "SELECT passenger_count, SUM(CASE WHEN payment_type = 1 THEN fare_cents ELSE 5 END) AS revenue FROM trips GROUP BY passenger_count",
        "SELECT passenger_count, SUM(CASE WHEN payment_type = 1 THEN fare_cents ELSE 0) AS revenue FROM trips GROUP BY passenger_count",
    ] {
        assert!(!sql(invalid).status.success(), "{invalid}");
    }
    fs::remove_dir_all(db).unwrap();
}
