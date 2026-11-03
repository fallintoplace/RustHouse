use std::fs;
use std::process::Command;

#[test]
fn streamed_three_table_join_groups_by_both_boroughs_and_orders_by_count() {
    let db = std::env::temp_dir().join(format!("rusthouse-taxi-three-join-group-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE trips (pickup_zone UInt64, dropoff_zone UInt64)",
        "CREATE TABLE zones (zone_id UInt64, borough String)",
        "INSERT INTO trips VALUES (1, 2), (1, 2), (2, 1), (1, 3), (3, 2), (4, 2)",
        "INSERT INTO zones VALUES (1, 'Manhattan'), (2, 'Brooklyn'), (3, 'Queens')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT p.borough, d.borough, COUNT(*) AS n FROM trips t JOIN zones p ON t.pickup_zone = p.zone_id JOIN zones d ON t.dropoff_zone = d.zone_id GROUP BY p.borough, d.borough ORDER BY n DESC, p.borough, d.borough LIMIT 10";
    let result = sql(query);
    assert!(result.status.success(), "{query}: {result:?}");
    assert_eq!(result.stdout, b"p.borough\td.borough\tn\nManhattan\tBrooklyn\t2\nBrooklyn\tManhattan\t1\nManhattan\tQueens\t1\nQueens\tBrooklyn\t1\n");
    let limited = "SELECT p.borough, d.borough, COUNT(*) AS n FROM trips t JOIN zones p ON t.pickup_zone = p.zone_id JOIN zones d ON t.dropoff_zone = d.zone_id GROUP BY p.borough, d.borough ORDER BY n DESC, p.borough, d.borough LIMIT 2 OFFSET 1";
    let result = sql(limited);
    assert!(result.status.success(), "{limited}: {result:?}");
    assert_eq!(result.stdout, b"p.borough\td.borough\tn\nBrooklyn\tManhattan\t1\nManhattan\tQueens\t1\n");
    let empty = "SELECT p.borough, d.borough, COUNT(*) AS n FROM trips t JOIN zones p ON t.pickup_zone = p.zone_id JOIN zones d ON t.dropoff_zone = d.zone_id GROUP BY p.borough, d.borough ORDER BY n DESC LIMIT 0";
    let result = sql(empty);
    assert!(result.status.success(), "{empty}: {result:?}");
    assert_eq!(result.stdout, b"p.borough\td.borough\tn\n");
    fs::remove_dir_all(db).unwrap();
}
