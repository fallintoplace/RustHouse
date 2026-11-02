use std::fs;
use std::process::Command;

#[test]
fn chained_count_applies_post_join_null_filters() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-chain-filtered-count-{}",
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
        "CREATE TABLE trips (id UInt64, flag Nullable(UInt64))",
        "CREATE TABLE zones (id UInt64, state Nullable(UInt64))",
        "CREATE TABLE boroughs (id UInt64, active Nullable(UInt64))",
        "INSERT INTO trips VALUES (1, NULL), (2, 2), (3, 3)",
        "INSERT INTO zones VALUES (1, NULL), (1, 5), (2, NULL), (3, 5)",
        "INSERT INTO boroughs VALUES (1, NULL), (1, 9), (2, 9), (3, NULL)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (predicate, count) in [
        ("r.active IS NULL", 3),
        ("r.active IS NOT NULL", 3),
        ("d.state IS NULL", 3),
        ("f.flag IS NULL", 4),
        ("f.flag IS NOT NULL", 2),
    ] {
        let query = format!(
            "SELECT COUNT(*) FROM trips f JOIN zones d ON f.id = d.id JOIN boroughs r ON d.id = r.id WHERE {predicate}"
        );
        let result = sql(&query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            format!("count\n{count}\n"),
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
