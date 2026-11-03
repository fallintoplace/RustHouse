use std::fs;
use std::process::Command;

#[test]
fn grouped_join_count_accepts_a_result_alias() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-taxi-grouped-join-count-alias-{}",
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
        "CREATE TABLE trips (pickup_zone UInt64)",
        "CREATE TABLE zones (zone_id UInt64, borough String)",
        "INSERT INTO trips VALUES (1), (2), (1), (3)",
        "INSERT INTO zones VALUES (1, 'Manhattan'), (2, 'Brooklyn'), (3, 'Brooklyn')",
    ] {
        let actual = sql(statement);
        assert!(actual.status.success(), "{statement}: {actual:?}");
    }
    let query = "SELECT z.borough, COUNT(*) AS n FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id GROUP BY z.borough ORDER BY z.borough";
    let actual = sql(query);
    assert!(actual.status.success(), "{query}: {actual:?}");
    assert_eq!(actual.stdout, b"z.borough\tn\nBrooklyn\t2\nManhattan\t2\n");

    let unaliased = "SELECT z.borough, COUNT(*) FROM trips t JOIN zones z ON t.pickup_zone = z.zone_id GROUP BY z.borough ORDER BY z.borough";
    let actual = sql(unaliased);
    assert!(actual.status.success(), "{unaliased}: {actual:?}");
    assert_eq!(actual.stdout, b"z.borough\tcount\nBrooklyn\t2\nManhattan\t2\n");
    fs::remove_dir_all(db).unwrap();
}
