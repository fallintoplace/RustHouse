use std::fs;
use std::process::Command;

#[test]
fn case_when_sum_combines_unsigned_fares_with_signed_defaults() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-case-mixed-sum-{}",
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
        "CREATE TABLE trips (amount UInt64, paid UInt8)",
        "INSERT INTO trips VALUES (10, 0), (20, 1), (30, 1), (40, 0)",
    ] {
        let actual = sql(statement);
        assert!(actual.status.success(), "{statement}: {actual:?}");
    }
    for (query, expected) in [
        (
            "SELECT SUM(CASE WHEN paid = 1 THEN amount ELSE 0 END) FROM trips",
            "50",
        ),
        (
            "SELECT SUM(CASE WHEN paid = 1 THEN amount ELSE -2 END) FROM trips",
            "46",
        ),
        (
            "SELECT SUM(CASE WHEN paid = 1 THEN -2 ELSE amount END) FROM trips",
            "46",
        ),
        (
            "SELECT SUM(amount) FROM trips",
            "100",
        ),
    ] {
        let actual = sql(query);
        assert!(actual.status.success(), "{query}: {actual:?}");
        assert_eq!(
            String::from_utf8(actual.stdout).unwrap().lines().last(),
            Some(expected),
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
