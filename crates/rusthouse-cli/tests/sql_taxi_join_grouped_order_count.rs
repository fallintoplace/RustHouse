use std::fs;
use std::process::Command;

#[test]
fn grouped_join_can_order_by_count_alias_and_break_ties_on_keys() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-taxi-grouped-sort-{}",
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
        "CREATE TABLE trips (pickup_zone UInt64, pickup_day UInt8)",
        "CREATE TABLE zones (zone_id UInt64, borough String)",
        "INSERT INTO trips VALUES (1, 1), (1, 1), (2, 1), (2, 1), (3, 2)",
        "INSERT INTO zones VALUES (1, 'Manhattan'), (2, 'Brooklyn'), (3, 'Queens')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (statement, expected) in [
        (
            "SELECT z.borough, COUNT(*) AS n FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id GROUP BY z.borough ORDER BY n DESC, z.borough LIMIT 2",
            b"z.borough\tn\nBrooklyn\t2\nManhattan\t2\n".as_slice(),
        ),
        (
            "SELECT t.pickup_day, z.borough, COUNT(*) AS n FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id GROUP BY t.pickup_day, z.borough ORDER BY n DESC, t.pickup_day, z.borough LIMIT 3",
            b"t.pickup_day\tz.borough\tn\n1\tBrooklyn\t2\n1\tManhattan\t2\n2\tQueens\t1\n".as_slice(),
        ),
        (
            "SELECT z.borough, COUNT(*) FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id GROUP BY z.borough ORDER BY COUNT(*) DESC, z.borough LIMIT 1",
            b"z.borough\tcount\nBrooklyn\t2\n".as_slice(),
        ),
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(result.stdout, expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
