use std::fs;
use std::process::Command;

#[test]
fn join_limit_zero_skips_large_builds_but_validates_columns() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-zero-fast-{}",
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
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimensions (id UInt64)",
        "INSERT INTO facts SELECT number FROM numbers(20000)",
        "INSERT INTO dimensions VALUES (1)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT f.id, d.id FROM facts f LEFT JOIN dimensions d ON f.id = d.id ORDER BY f.id LIMIT 0",
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"f.id\td.id\n");

    let wrong = sql(
        "SELECT f.id FROM facts f JOIN dimensions d ON f.missing = d.id LIMIT 0",
    );
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("unknown JOIN column"));
    fs::remove_dir_all(db).unwrap();
}
