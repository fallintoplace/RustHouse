use std::fs;
use std::process::Command;

#[test]
fn grouped_sums_over_two_table_inner_joins() {
    let db = std::env::temp_dir().join(format!("rusthouse-two-table-sum-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE a (id UInt64, amount Nullable(Int64))",
        "CREATE TABLE b (id UInt64, category String, weight UInt64)",
        "INSERT INTO a VALUES (1, 3), (1, -1), (2, NULL), (3, 8)",
        "INSERT INTO b VALUES (1, 'x', 2), (2, 'y', 3), (3, 'z', 1)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id GROUP BY b.category ORDER BY b.category",
            "b.category\ttotal\nx\t2\ny\t\\N\nz\t8\n",
        ),
        (
            "SELECT b.category, SUM(amount * weight) AS total FROM a JOIN b ON a.id = b.id WHERE b.weight >= 2 GROUP BY b.category ORDER BY b.category",
            "b.category\ttotal\nx\t4\ny\t\\N\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id GROUP BY b.category ORDER BY total DESC NULLS LAST LIMIT 1 OFFSET 1",
            "b.category\ttotal\nx\t2\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id WHERE b.weight = 99 GROUP BY b.category",
            "b.category\ttotal\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id GROUP BY b.category LIMIT 0",
            "b.category\ttotal\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id WHERE b.weight IN (2, 3) GROUP BY b.category",
        "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id WHERE b.category LIKE 'x%' GROUP BY b.category",
        "SELECT b.category, SUM(a.amount) AS total FROM a LEFT JOIN b ON a.id = b.id GROUP BY b.category",
        "SELECT b.category, SUM(missing) AS total FROM a JOIN b ON a.id = b.id GROUP BY b.category LIMIT 0",
        "SELECT b.category, SUM(b.category) AS total FROM a JOIN b ON a.id = b.id GROUP BY b.category LIMIT 0",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
    assert!(sql("INSERT INTO b VALUES (1, 'x', 2)").status.success());
    let duplicate = sql(
        "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id WHERE b.weight >= 2 AND a.amount > 0 GROUP BY b.category ORDER BY b.category",
    );
    assert!(duplicate.status.success(), "{duplicate:?}");
    assert_eq!(duplicate.stdout, b"b.category\ttotal\nx\t6\n");
    let any = sql(
        "SELECT b.category, SUM(a.amount) AS total FROM a ANY INNER JOIN b ON a.id = b.id GROUP BY b.category ORDER BY b.category",
    );
    assert!(any.status.success(), "{any:?}");
    assert_eq!(any.stdout, b"b.category\ttotal\nx\t2\ny\t\\N\nz\t8\n");
    fs::remove_dir_all(db).unwrap();
}
