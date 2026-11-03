use std::fs;
use std::process::Command;

#[test]
fn conditional_scalar_sum_checks_types_and_null_conditions() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-case-sum-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE sales (id UInt64, price Nullable(Float64), discount Float64)",
        "CREATE TABLE items (id UInt64, kind Nullable(String), factor UInt64)",
        "INSERT INTO sales VALUES (1, 100, 0.1), (2, 50, 0.2), (3, NULL, 0.5), (4, 20, 0)",
        "INSERT INTO items VALUES (1, 'PROMO A', 2), (2, 'OTHER', 1), (3, 'PROMO B', 1), (4, NULL, 2)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT SUM(CASE WHEN kind LIKE 'PROMO%' THEN price * (1 - discount) ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id",
            "total\n90\n",
        ),
        (
            "SELECT SUM(CASE WHEN kind NOT LIKE 'PROMO%' THEN price ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id",
            "total\n50\n",
        ),
        (
            "SELECT SUM(CASE WHEN factor > 1 AND kind LIKE 'PROMO%' THEN price ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id",
            "total\n100\n",
        ),
        (
            "SELECT SUM(CASE WHEN kind LIKE 'PROMO%' THEN price END) AS total FROM sales s JOIN items i ON s.id = i.id",
            "total\n100\n",
        ),
        (
            "SELECT SUM(CASE WHEN kind LIKE 'PROMO%' OR factor = 2 THEN price ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id",
            "total\n120\n",
        ),
        (
            "SELECT SUM(CASE WHEN kind LIKE 'PROMO%' AND factor = 2 THEN price ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id",
            "total\n100\n",
        ),
        (
            "SELECT SUM(CASE WHEN kind LIKE 'PROMO%' THEN price WHEN factor = 2 THEN price * 2 ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id",
            "total\n140\n",
        ),
        (
            "SELECT SUM(CASE WHEN factor > 0 THEN 1 ELSE 1 / 0 END) AS total FROM sales s JOIN items i ON s.id = i.id",
            "total\n4\n",
        ),
        (
            "SELECT SUM(CASE WHEN kind LIKE 'PROMO%' THEN price ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id LIMIT 0",
            "total\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT SUM(CASE WHEN price LIKE 'x%' THEN 1 ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id LIMIT 0",
        "SELECT SUM(CASE WHEN kind LIKE 'x%' THEN kind ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id LIMIT 0",
        "SELECT SUM(CASE WHEN missing LIKE 'x%' THEN 1 ELSE 0 END) AS total FROM sales s JOIN items i ON s.id = i.id LIMIT 0",
    ] {
        let result = sql(query);
        assert!(!result.status.success(), "{query}: {result:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
