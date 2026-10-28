use std::fs;
use std::process::Command;

#[test]
fn join_offset_without_limit_returns_remaining_rows() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-join-offset-only-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimension (id UInt64)",
        "INSERT INTO facts VALUES (1), (2), (3)",
        "INSERT INTO dimension VALUES (1), (2), (3)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result =
        sql("SELECT f.id FROM facts f JOIN dimension d ON f.id = d.id ORDER BY f.id OFFSET 1");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"f.id\n2\n3\n");
    fs::remove_dir_all(db).unwrap();
}
