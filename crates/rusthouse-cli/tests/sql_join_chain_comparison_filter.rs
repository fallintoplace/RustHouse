use std::fs;
use std::process::Command;

#[test]
fn chained_inner_join_filters_fact_and_dimension_comparisons_before_ordering() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-chained-comparison-{}",
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
        "CREATE TABLE trips (id UInt64, pickup_zone UInt64, fare_cents UInt64)",
        "CREATE TABLE pickups (id UInt64, borough UInt64)",
        "CREATE TABLE boroughs (id UInt64, label String)",
        "INSERT INTO trips VALUES (1,10,9000), (2,10,7000), (3,20,8000), (4,30,4000), (5,10,6000), (6,99,20000)",
        "INSERT INTO pickups VALUES (10,1), (10,1), (20,2), (30,1)",
        "INSERT INTO boroughs VALUES (1,'Manhattan'), (2,'Queens')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT t.id, b.label FROM trips t JOIN pickups p ON t.pickup_zone = p.id JOIN boroughs b ON p.borough = b.id WHERE t.fare_cents >= 7000 ORDER BY t.fare_cents DESC, t.id ASC LIMIT 4",
            "t.id\tb.label\n1\tManhattan\n1\tManhattan\n3\tQueens\n2\tManhattan\n",
        ),
        (
            "SELECT t.id, b.label FROM trips t JOIN pickups p ON t.pickup_zone = p.id JOIN boroughs b ON p.borough = b.id WHERE b.id = 1 ORDER BY t.id LIMIT 3",
            "t.id\tb.label\n1\tManhattan\n1\tManhattan\n2\tManhattan\n",
        ),
        (
            "SELECT t.id, b.label FROM trips t JOIN pickups p ON t.pickup_zone = p.id JOIN boroughs b ON p.borough = b.id WHERE p.borough > 1 ORDER BY t.id",
            "t.id\tb.label\n3\tQueens\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }
    let empty = sql("SELECT t.id, b.label FROM trips t JOIN pickups p ON t.pickup_zone = p.id JOIN boroughs b ON p.borough = b.id WHERE t.fare_cents > 99999 ORDER BY t.id LIMIT 2");
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"t.id\tb.label\n");
    fs::remove_dir_all(db).unwrap();
}
