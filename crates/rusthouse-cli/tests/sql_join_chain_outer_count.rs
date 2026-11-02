use std::fs;
use std::process::Command;

#[test]
fn chained_count_includes_outer_join_null_extended_rows() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-chained-outer-count-{}",
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
        "CREATE TABLE trips (id UInt64)",
        "CREATE TABLE zones (id UInt64)",
        "CREATE TABLE boroughs (id UInt64)",
        "INSERT INTO trips VALUES (1), (2), (3)",
        "INSERT INTO zones VALUES (1), (1), (4)",
        "INSERT INTO boroughs VALUES (1), (7)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, count) in [
        (
            "SELECT COUNT(*) FROM trips t LEFT JOIN zones z ON t.id = z.id LEFT JOIN boroughs b ON z.id = b.id",
            4,
        ),
        (
            "SELECT COUNT(*) FROM trips t LEFT JOIN zones z ON t.id = z.id LEFT JOIN boroughs b ON z.id = b.id WHERE b.id IS NULL",
            2,
        ),
        (
            "SELECT COUNT(*) FROM trips t JOIN zones z ON t.id = z.id FULL JOIN boroughs b ON z.id = b.id",
            3,
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            format!("count\n{count}\n"),
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
