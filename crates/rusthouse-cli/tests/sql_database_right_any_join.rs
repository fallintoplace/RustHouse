use std::fs;
use std::process::Command;

#[test]
fn right_any_join_keeps_first_left_match_for_each_right_row() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-right-any-{}",
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
        "CREATE TABLE facts (id UInt64, k UInt64)",
        "CREATE TABLE dimensions (id UInt64, k UInt64)",
        "INSERT INTO facts VALUES (1, 10), (2, 10), (3, 30)",
        "INSERT INTO dimensions VALUES (7, 10), (8, 10), (9, 20)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for join in ["RIGHT ANY JOIN", "ANY RIGHT JOIN"] {
        let query = format!(
            "SELECT f.id, d.id FROM facts f {join} dimensions d ON f.k = d.k ORDER BY d.id"
        );
        let result = sql(&query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, b"f.id\td.id\n1\t7\n1\t8\n\\N\t9\n");
    }
    fs::remove_dir_all(db).unwrap();
}
