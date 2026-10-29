use std::fs;
use std::process::Command;

#[test]
fn count_over_join_chains_handles_duplicates_outer_rows_and_pagination() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-chain-count-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE orders (id UInt64, customer_id Nullable(UInt64))",
        "CREATE TABLE customers (id UInt64, region_id UInt64)",
        "CREATE TABLE regions (id UInt64, label String)",
        "INSERT INTO orders VALUES (1, 10), (2, 10), (3, 20), (4, NULL)",
        "INSERT INTO customers VALUES (10, 1), (10, 2), (20, 3)",
        "INSERT INTO regions VALUES (1, 'Europe'), (2, 'Asia'), (3, 'America')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let cases: &[(&str, &[u8])] = &[
        (
            "SELECT COUNT(*) FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id",
            b"count\n5\n",
        ),
        (
            "SELECT COUNT(*) FROM orders o LEFT JOIN customers c ON o.customer_id = c.id LEFT JOIN regions r ON c.region_id = r.id",
            b"count\n6\n",
        ),
        (
            "SELECT COUNT(*) FROM orders o LEFT JOIN customers c ON o.customer_id = c.id LEFT JOIN regions r ON c.region_id = r.id WHERE r.label IS NULL",
            b"count\n1\n",
        ),
        (
            "SELECT COUNT(*) FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id LIMIT 0",
            b"count\n",
        ),
        (
            "SELECT COUNT(*) FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id OFFSET 1",
            b"count\n",
        ),
    ];
    for &(statement, expected) in cases {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
        assert_eq!(output.stdout, expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
