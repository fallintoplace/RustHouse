use std::fs;
use std::process::Command;

#[test]
fn taxi_top_trips_preserve_and_or_precedence_and_tie_order() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-taxi-top-or-{}",
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
        "CREATE TABLE trips (trip_id UInt64, fare_cents UInt64, distance_milli UInt64, pickup_zone UInt64)",
        "INSERT INTO trips VALUES (1,10000,25000,1), (2,7000,9000,999), (3,9000,11000,2), (4,3000,4000,999), (5,6000,15000,3), (6,2500,16000,4), (7,9000,11000,1)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let explicit = sql(
        "SELECT trip_id, fare_cents FROM trips WHERE (fare_cents >= 5000 AND distance_milli >= 10000) OR pickup_zone = 999 ORDER BY fare_cents DESC, trip_id ASC LIMIT 3 OFFSET 1"
    );
    assert!(explicit.status.success(), "{explicit:?}");
    assert_eq!(explicit.stdout, b"trip_id\tfare_cents\n3\t9000\n7\t9000\n2\t7000\n");

    let implicit = sql(
        "SELECT trip_id, fare_cents FROM trips WHERE fare_cents >= 5000 AND distance_milli >= 10000 OR pickup_zone = 999 ORDER BY fare_cents DESC, trip_id ASC LIMIT 3 OFFSET 1"
    );
    assert!(implicit.status.success(), "{implicit:?}");
    assert_eq!(implicit.stdout, explicit.stdout);

    fs::remove_dir_all(db).unwrap();
}
