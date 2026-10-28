use std::fs;
use std::process::Command;

#[test]
fn parenthesized_composite_join_predicates_execute() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-on-paren-{}",
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
        "CREATE TABLE orders (id UInt64, tenant UInt64, customer UInt64)",
        "CREATE TABLE customers (tenant UInt64, customer UInt64, name String)",
        "INSERT INTO orders VALUES (1, 1, 7), (2, 2, 7)",
        "INSERT INTO customers VALUES (1, 7, 'Ada'), (2, 7, 'Bob')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT o.id, c.name FROM orders o JOIN customers c ON (o.tenant = c.tenant AND (o.customer = c.customer)) ORDER BY o.id",
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"o.id\tc.name\n1\tAda\n2\tBob\n");
    fs::remove_dir_all(db).unwrap();
}
