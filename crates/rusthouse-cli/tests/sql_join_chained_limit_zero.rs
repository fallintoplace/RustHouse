use std::fs;
use std::process::Command;

#[test]
fn chained_limit_zero_validates_columns_without_scanning_large_inputs() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-chain-zero-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
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
        "CREATE TABLE dims (id UInt64)",
        "CREATE TABLE tags (id UInt64)",
        "INSERT INTO facts SELECT number FROM numbers(20000)",
        "INSERT INTO dims VALUES (3)",
        "INSERT INTO tags VALUES (3)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let zero = sql(
        "SELECT f.id, t.id FROM facts f JOIN dims d ON f.id = d.id LEFT JOIN tags t ON d.id = t.id ORDER BY t.id LIMIT 0",
    );
    assert!(zero.status.success(), "{zero:?}");
    assert_eq!(zero.stdout, b"f.id\tt.id\n");

    let invalid = sql(
        "SELECT f.id, t.missing FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id LIMIT 0",
    );
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unknown JOIN column"));
    fs::remove_dir_all(db).unwrap();
}
