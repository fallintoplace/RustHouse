use std::fs;
use std::process::Command;

#[test]
fn correlated_pair_sum_uses_both_keys_and_filters() {
    let db = std::env::temp_dir().join(format!("rusthouse-pair-sum-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE stock (part UInt64, supplier UInt64, available Nullable(Int64))",
        "CREATE TABLE parts (id UInt64)",
        "CREATE TABLE sales (part_key UInt64, supplier_key UInt64, quantity Nullable(Float64), day Date)",
        "INSERT INTO parts VALUES (1), (2), (3)",
        "INSERT INTO stock VALUES (1, 1, 11), (1, 1, 10), (1, 2, 3), (2, 1, 5), (3, 1, NULL)",
        "INSERT INTO sales VALUES (1, 1, 10, '1994-01-01'), (1, 1, 10, '1994-12-31'), (1, 1, 999, '1995-01-01'), (1, 2, 4, '1994-06-01'), (2, 1, NULL, '1994-06-01'), (3, 1, 1, '1994-06-01')",
    ] {
        let result = sql(statement);
        assert!(
            result.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let query = "SELECT stock.part, stock.supplier, stock.available FROM stock JOIN parts ON stock.part = parts.id WHERE stock.available > (SELECT 0.5 * SUM(s.quantity) FROM sales s WHERE s.part_key = stock.part AND s.supplier_key = stock.supplier AND s.day >= CAST('1994-01-01' AS date) AND s.day < CAST('1995-01-01' AS date)) ORDER BY stock.part, stock.supplier, stock.available";
    let result = sql(query);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout,
        b"stock.part\tstock.supplier\tstock.available\n1\t1\t11\n1\t2\t3\n"
    );
    let limited = query.to_owned() + " LIMIT 1 OFFSET 1";
    let result = sql(&limited);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout,
        b"stock.part\tstock.supplier\tstock.available\n1\t2\t3\n"
    );
    let zero = query.to_owned() + " LIMIT 0";
    let result = sql(&zero);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout,
        b"stock.part\tstock.supplier\tstock.available\n"
    );
    for invalid in [
        query.replace(
            "s.supplier_key = stock.supplier",
            "s.part_key = stock.supplier",
        ),
        query.replace("SUM(s.quantity)", "SUM(s.missing)"),
        query.replace("s.day >=", "stock.available >="),
        query.replace("s.day >=", "s.part_key >="),
        query.replace("JOIN parts ON", "LEFT JOIN parts ON"),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    for statement in [
        "CREATE TABLE large_stock (part UInt64, supplier UInt64, available UInt64)",
        "CREATE TABLE large_sales (part_key UInt64, supplier_key UInt64, quantity UInt64)",
        "INSERT INTO large_stock VALUES (1, 1, 9007199254740993)",
        "INSERT INTO large_sales VALUES (1, 1, 9007199254740993)",
    ] {
        let result = sql(statement);
        assert!(
            result.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let exact = "SELECT large_stock.available FROM large_stock JOIN parts ON large_stock.part = parts.id WHERE large_stock.available > (SELECT 1 * SUM(s.quantity) FROM large_sales s WHERE s.part_key = large_stock.part AND s.supplier_key = large_stock.supplier)";
    let result = sql(exact);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"large_stock.available\n");
    fs::remove_dir_all(db).unwrap();
}
