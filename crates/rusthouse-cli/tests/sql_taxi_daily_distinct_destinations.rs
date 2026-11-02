use std::fs;
use std::process::Command;

#[test]
fn taxi_distinct_destinations_by_day_having_ranks_non_null_values() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-taxi-distinct-day-{}",
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
        "CREATE TABLE trips (pickup_day UInt32, dropoff_zone Nullable(UInt32))",
        "INSERT INTO trips VALUES (1,5), (1,5), (1,6), (1,NULL), (2,7), (2,8), (2,9), (2,8), (3,1), (3,NULL), (4,10), (4,11), (4,12), (4,13), (5,NULL)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT pickup_day, COUNT(DISTINCT dropoff_zone) AS destinations FROM trips GROUP BY pickup_day HAVING COUNT(DISTINCT dropoff_zone) >= 2 ORDER BY count DESC, pickup_day ASC LIMIT 3";
    let result = sql(query);
    assert!(result.status.success(), "{query}: {result:?}");
    assert_eq!(
        result.stdout,
        b"pickup_day\tdestinations\n4\t4\n2\t3\n1\t2\n"
    );

    let empty = sql("SELECT pickup_day, COUNT(DISTINCT dropoff_zone) AS destinations FROM trips GROUP BY pickup_day HAVING COUNT(DISTINCT dropoff_zone) >= 5 ORDER BY count DESC LIMIT 3");
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"pickup_day\tdestinations\n");
    fs::remove_dir_all(db).unwrap();
}
