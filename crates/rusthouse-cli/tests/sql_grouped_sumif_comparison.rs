use std::fs;
use std::process::Command;

#[test]
fn grouped_sumif_accepts_typed_numeric_comparisons() {
    let db = std::env::temp_dir().join(format!("rusthouse-taxi-grouped-sumif-{}", std::process::id()));
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
    let cases = [
        ("SELECT passenger_count, SUMIF(fare_cents, payment_type = 1) AS revenue FROM trips GROUP BY passenger_count ORDER BY passenger_count", b"passenger_count\trevenue\n1\t1300\n2\t800\n".as_slice()),
        ("SELECT passenger_count, SUMIF(fare_cents, payment_type > 1) AS revenue FROM trips GROUP BY passenger_count ORDER BY passenger_count", b"passenger_count\trevenue\n1\t2000\n2\t5000\n".as_slice()),
        ("SELECT passenger_count, SUMIF(fare_cents, payment_type) AS revenue FROM trips GROUP BY passenger_count ORDER BY passenger_count", b"passenger_count\trevenue\n1\t3300\n2\t5800\n".as_slice()),
    ];
    for (statement, expected) in cases {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(result.stdout, expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
