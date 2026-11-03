use std::fs;
use std::process::Command;

#[test]
fn scalar_expression_sum_streams_inner_join_rows() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-scalar-sum-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE sales (id UInt64, price Nullable(Float64), discount Float64)",
        "CREATE TABLE items (id UInt64, kind String, factor UInt64)",
        "CREATE TABLE flags (id UInt64, active UInt64)",
        "INSERT INTO sales VALUES (1, 100, 0.1), (2, 200, 0.25), (3, NULL, 0.5)",
        "INSERT INTO items VALUES (1, 'a', 1), (1, 'b', 1), (2, 'a', 2), (3, 'a', 1)",
        "INSERT INTO flags VALUES (1, 1), (2, 1), (3, 0)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT SUM(price * factor) AS total FROM sales CROSS JOIN items",
            "total\n1500\n",
        ),
        (
            "SELECT SUM(price * factor) FROM sales s JOIN items i ON s.id = i.id",
            "sum(price * factor)\n600\n",
        ),
        (
            "SELECT SUM(price * (1 - discount)) AS total FROM sales s JOIN items i ON s.id = i.id",
            "total\n330\n",
        ),
        (
            "SELECT SUM(price * (1 - discount)) AS total FROM sales s ANY INNER JOIN items i ON s.id = i.id",
            "total\n240\n",
        ),
        (
            "SELECT SUM(price * (1 - discount)) AS total FROM sales s JOIN items i ON s.id = i.id WHERE kind = 'a' AND factor > 0",
            "total\n240\n",
        ),
        (
            "SELECT SUM(price * factor) AS total FROM sales s JOIN items i ON s.id = i.id JOIN flags f ON s.id = f.id WHERE active = 1",
            "total\n600\n",
        ),
        (
            "SELECT SUM(price * factor) AS total FROM sales s JOIN items i ON s.id = i.id WHERE kind = 'none'",
            "total\n\\N\n",
        ),
        (
            "SELECT SUM(price * factor) AS total FROM sales s JOIN items i ON s.id = i.id WHERE s.id = 3",
            "total\n\\N\n",
        ),
        (
            "SELECT SUM(price * factor) AS total FROM sales s JOIN items i ON s.id = i.id LIMIT 0",
            "total\n",
        ),
        (
            "SELECT SUM(price * factor) AS total FROM sales s JOIN items i ON s.id = i.id OFFSET 1",
            "total\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT SUM(price * missing) AS total FROM sales s JOIN items i ON s.id = i.id LIMIT 0",
        "SELECT SUM(id + 1) AS total FROM sales s JOIN items i ON s.id = i.id LIMIT 0",
        "SELECT SUM(price * kind) AS total FROM sales s JOIN items i ON s.id = i.id LIMIT 0",
        "SELECT SUM(price / 0) AS total FROM sales s JOIN items i ON s.id = i.id",
        "SELECT SUM(price * factor) AS total FROM sales s LEFT JOIN items i ON s.id = i.id",
        "SELECT SUM(price * factor) AS total FROM sales s JOIN items i ON s.id = i.id WHERE kind IN ('a')",
    ] {
        let result = sql(query);
        assert!(!result.status.success(), "{query}: {result:?}");
    }
    let result = sql("TRUNCATE TABLE items");
    assert!(result.status.success(), "{result:?}");
    let result =
        sql("SELECT SUM(price * factor) AS total FROM sales s JOIN items i ON s.id = i.id");
    assert_eq!(result.stdout, b"total\n\\N\n");
    fs::remove_dir_all(db).unwrap();
}
