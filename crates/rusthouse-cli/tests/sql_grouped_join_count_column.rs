use std::fs;
use std::process::Command;

#[test]
fn grouped_count_column_preserves_zero_groups_in_left_joins() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-group-count-column-{}",
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
        "CREATE TABLE a (id UInt64, k Nullable(UInt64), v Nullable(UInt64))",
        "CREATE TABLE b (k UInt64, v Nullable(UInt64))",
        "INSERT INTO a VALUES (1, 1, 9), (2, 2, NULL), (3, 3, 3), (4, NULL, NULL)",
        "INSERT INTO b VALUES (1, NULL), (1, 7), (1, 8), (2, NULL)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT a.id, COUNT(b.v) AS n FROM a LEFT JOIN b ON a.k = b.k GROUP BY a.id ORDER BY a.id",
            "a.id\tn\n1\t2\n2\t0\n3\t0\n4\t0\n",
        ),
        (
            "SELECT a.id, COUNT(a.v) AS n FROM a LEFT JOIN b ON a.k = b.k GROUP BY a.id ORDER BY a.id",
            "a.id\tn\n1\t3\n2\t0\n3\t1\n4\t0\n",
        ),
        (
            "SELECT a.id, COUNT(b.v) AS n FROM a JOIN b ON a.k = b.k GROUP BY a.id ORDER BY a.id",
            "a.id\tn\n1\t2\n2\t0\n",
        ),
        (
            "SELECT b.k, COUNT(a.v) AS n FROM a JOIN b ON a.k = b.k GROUP BY b.k ORDER BY b.k",
            "b.k\tn\n1\t3\n2\t0\n",
        ),
        (
            "SELECT b.v, COUNT(b.v) AS n FROM a LEFT JOIN b ON a.k = b.k GROUP BY b.v ORDER BY b.v NULLS LAST",
            "b.v\tn\n7\t1\n8\t1\n\\N\t0\n",
        ),
        (
            "SELECT a.id, COUNT(a.v) AS n FROM a ANY LEFT JOIN b ON a.k = b.k GROUP BY a.id ORDER BY a.id",
            "a.id\tn\n1\t1\n2\t0\n3\t1\n4\t0\n",
        ),
        (
            "SELECT a.id, COUNT(b.v) AS n FROM a LEFT JOIN b ON a.k = b.k GROUP BY a.id ORDER BY n DESC, a.id LIMIT 2 OFFSET 1",
            "a.id\tn\n2\t0\n3\t0\n",
        ),
        (
            "SELECT a.id, COUNT(b.v) FROM a LEFT JOIN b ON a.k = b.k GROUP BY a.id LIMIT 0",
            "a.id\tcount(b.v)\n",
        ),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT a.id, COUNT(b.missing) FROM a LEFT JOIN b ON a.k = b.k GROUP BY a.id LIMIT 0",
        "SELECT a.id, COUNT(other.v) FROM a LEFT JOIN b ON a.k = b.k GROUP BY a.id",
        "SELECT a.id, COUNT(b.v) AS n FROM a RIGHT JOIN b ON a.k = b.k GROUP BY a.id",
        "SELECT a.id, COUNT(b.v) AS n FROM a LEFT JOIN b ON a.k = b.k GROUP BY a.id ORDER BY COUNT(*)",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
