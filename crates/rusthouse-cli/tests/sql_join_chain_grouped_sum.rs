use std::fs;
use std::process::Command;

#[test]
fn chained_sum_groups_nullable_values_and_orders_alias() {
    let db = std::env::temp_dir().join(format!("rusthouse-chain-sum-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE a (id UInt64, amount Nullable(Int64))",
        "CREATE TABLE b (id UInt64, next UInt64, category String)",
        "CREATE TABLE c (id UInt64, flag UInt64)",
        "INSERT INTO a VALUES (1, 3), (2, -2), (3, NULL), (4, 5)",
        "INSERT INTO b VALUES (1, 10, 'x'), (2, 10, 'x'), (3, 10, 'z'), (4, 11, 'y')",
        "INSERT INTO c VALUES (10, 1), (11, 2)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING total = 1 ORDER BY b.category",
            "b.category\ttotal\nx\t1\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING total != 1 ORDER BY b.category",
            "b.category\ttotal\ny\t5\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING total < 5 ORDER BY b.category",
            "b.category\ttotal\nx\t1\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING total >= 5 ORDER BY b.category",
            "b.category\ttotal\ny\t5\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING SUM(a.amount) > 1 ORDER BY total DESC",
            "b.category\ttotal\ny\t5\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING total >= -2.5 ORDER BY total DESC LIMIT 1 OFFSET 1",
            "b.category\ttotal\nx\t1\n",
        ),
        (
            "SELECT b.category, SUM(amount + flag) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING SUM(amount + flag) <= 3 ORDER BY b.category",
            "b.category\ttotal\nx\t3\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category ORDER BY b.category",
            "b.category\ttotal\nx\t1\ny\t5\nz\t\\N\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category ORDER BY total DESC NULLS LAST LIMIT 1",
            "b.category\ttotal\ny\t5\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category ORDER BY total DESC NULLS LAST LIMIT 1 OFFSET 1",
            "b.category\ttotal\nx\t1\n",
        ),
        (
            "SELECT b.category, c.flag, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE c.flag = 1 GROUP BY b.category, c.flag ORDER BY b.category",
            "b.category\tc.flag\ttotal\nx\t1\t1\nz\t1\t\\N\n",
        ),
        (
            "SELECT b.category, b.category, COUNT(*) AS n FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category, b.category ORDER BY b.category",
            "b.category\tb.category\tn\nx\tx\t2\ny\ty\t1\nz\tz\t1\n",
        ),
        (
            "SELECT b.category, c.flag, COUNT(*) AS n FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY c.flag, b.category ORDER BY n DESC, b.category",
            "b.category\tc.flag\tn\nx\t1\t2\ny\t2\t1\nz\t1\t1\n",
        ),
        (
            "SELECT b.category, SUM(amount + flag) AS total, c.flag FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY c.flag, b.category ORDER BY total DESC NULLS LAST LIMIT 1",
            "b.category\ttotal\tc.flag\ny\t7\t2\n",
        ),
        (
            "SELECT b.category, SUM(amount + flag) AS total, c.flag FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY c.flag, b.category ORDER BY b.category",
            "b.category\ttotal\tc.flag\nx\t3\t1\ny\t7\t2\nz\t\\N\t1\n",
        ),
        (
            "SELECT b.category, SUM(amount + flag) AS total, c.flag FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category, c.flag ORDER BY b.category",
            "b.category\ttotal\tc.flag\nx\t3\t1\ny\t7\t2\nz\t\\N\t1\n",
        ),
        (
            "SELECT b.category, SUM(amount + flag) AS total, c.flag FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category, c.flag LIMIT 0",
            "b.category\ttotal\tc.flag\n",
        ),
        (
            "SELECT b.category, SUM(amount + flag) AS total, c.flag FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE c.flag = 99 GROUP BY b.category, c.flag",
            "b.category\ttotal\tc.flag\n",
        ),
        (
            "SELECT b.category, SUM(amount * (1 - 0.25)) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category ORDER BY b.category",
            "b.category\ttotal\nx\t0.75\ny\t3.75\nz\t\\N\n",
        ),
        (
            "SELECT b.category, SUM(amount + flag) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category ORDER BY b.category",
            "b.category\ttotal\nx\t3\ny\t7\nz\t\\N\n",
        ),
        (
            "SELECT b.category, SUM(a.amount) FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE c.flag = 99 GROUP BY b.category",
            "b.category\tsum(a.amount)\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT b.category, SUM(a.amount) AS category FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING category > 0",
        "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING SUM(c.flag) > 0 LIMIT 0",
        "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING missing > 0 LIMIT 0",
        "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING total > '3' LIMIT 0",
        "SELECT b.category, SUM(b.category) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category LIMIT 0",
        "SELECT b.category, SUM(missing) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category LIMIT 0",
        "SELECT b.category, SUM(id) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category LIMIT 0",
        "SELECT b.category, SUM(a.amount) AS total FROM a LEFT JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category",
        "SELECT b.category, SUM(a.amount + 1) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category",
        "SELECT b.category, SUM(category + 1) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category LIMIT 0",
        "SELECT b.category, SUM(amount) AS total, c.flag FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category, b.category",
        "SELECT b.category, SUM(amount) AS total, c.flag FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY c.flag",
        "SELECT b.category, SUM(amount) AS total, c.flag, SUM(flag) AS more FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category, c.flag",
        "SELECT b.category, SUM(amount) AS total, c.flag, COUNT(*) FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category, c.flag",
        "SELECT b.category, SUM(amount / 0) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category",
        "SELECT b.category, SUM(id + 1) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category LIMIT 0",
    ] {
        let result = sql(query);
        assert!(!result.status.success(), "{query}: {result:?}");
    }
    let explained = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT b.category, SUM(a.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category HAVING total > 1", "--database"])
        .arg(&db)
        .arg("--explain")
        .output()
        .unwrap();
    assert!(explained.status.success(), "{explained:?}");
    assert!(String::from_utf8_lossy(&explained.stdout).contains("Having: SUM Gt 1"));
    let output = sql("TRUNCATE TABLE c");
    assert!(output.status.success(), "{output:?}");
    let output = sql(
        "SELECT b.category, SUM(amount) AS total, c.flag FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id GROUP BY b.category, c.flag",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"b.category\ttotal\tc.flag\n");
    fs::remove_dir_all(db).unwrap();
}
