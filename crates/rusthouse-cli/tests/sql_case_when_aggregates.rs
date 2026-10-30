use std::fs;
use std::process::Command;

#[test]
fn case_when_supports_conditional_numeric_aggregates() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-case-aggregate-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (amount Nullable(Int64), flag Int64)",
        "INSERT INTO events VALUES (10, 1), (20, 0), (NULL, 1), (-5, 1)",
    ] {
        let actual = sql(statement);
        assert!(actual.status.success(), "{statement}: {actual:?}");
    }
    for (query, expected) in [
        (
            "SELECT SUM(CASE WHEN flag > 0 THEN amount ELSE 0 END) FROM events",
            "5",
        ),
        (
            "SELECT SUM(CASE WHEN flag > 0 THEN 1 ELSE 0 END) FROM events",
            "3",
        ),
        (
            "SELECT SUM(CASE WHEN amount > 0 THEN amount WHEN amount < 0 THEN 0 ELSE 2 END) FROM events",
            "32",
        ),
        (
            "SELECT SUM(CASE WHEN flag > 0 THEN 1 END) FROM events",
            "3",
        ),
    ] {
        let actual = sql(query);
        assert!(actual.status.success(), "{query}: {actual:?}");
        let output = String::from_utf8(actual.stdout).unwrap();
        assert_eq!(output.lines().last(), Some(expected), "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
