use std::fs;
use std::process::Command;

#[test]
fn join_order_by_uses_secondary_columns_to_break_ties() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-join-multi-order-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, k UInt64)",
        "CREATE TABLE dimension (k UInt64, label String)",
        "INSERT INTO facts VALUES (1, 10), (2, 10), (3, 10)",
        "INSERT INTO dimension VALUES (10, 'b'), (10, 'a')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT f.id, d.label FROM facts f JOIN dimension d ON f.k = d.k ORDER BY d.label ASC, f.id DESC",
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"f.id\td.label\n3\ta\n2\ta\n1\ta\n3\tb\n2\tb\n1\tb\n"
    );
    fs::remove_dir_all(db).unwrap();
}
