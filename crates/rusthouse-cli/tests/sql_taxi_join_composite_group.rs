use std::fs;
use std::process::Command;

#[test]
fn grouped_join_counts_two_keys_and_sorts_them() {
    let db = std::env::temp_dir().join(format!("rusthouse-taxi-two-key-join-{}", std::process::id()));
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
        "INSERT INTO trips VALUES (1, 1), (1, 1), (2, 1), (3, 2), (4, 2), (5, 2)",
        "INSERT INTO zones VALUES (1, 'Manhattan'), (2, 'Manhattan'), (3, 'Queens'), (4, 'Queens')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT t.pickup_day, z.borough, COUNT(*) AS n FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id GROUP BY t.pickup_day, z.borough ORDER BY t.pickup_day, z.borough";
    let result = sql(query);
    assert!(result.status.success(), "{query}: {result:?}");
    assert_eq!(result.stdout, b"t.pickup_day\tz.borough\tn\n1\tManhattan\t3\n2\tQueens\t2\n");

    let duplicate = "SELECT z.borough, t.pickup_day, COUNT(*) FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id GROUP BY z.borough, t.pickup_day ORDER BY z.borough, t.pickup_day";
    let result = sql(duplicate);
    assert!(result.status.success(), "{duplicate}: {result:?}");
    assert_eq!(result.stdout, b"z.borough\tt.pickup_day\tcount\nManhattan\t1\t3\nQueens\t2\t2\n");

    for invalid in [
        "SELECT t.pickup_day, z.borough, COUNT(*) FROM trips t LEFT JOIN zones z ON t.pickup_zone = z.zone_id GROUP BY t.pickup_day, z.borough",
        "SELECT t.pickup_day, z.borough, COUNT(*) FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id GROUP BY z.borough, t.pickup_day",
    ] {
        assert!(!sql(invalid).status.success(), "{invalid}");
    }
    fs::remove_dir_all(db).unwrap();
}
