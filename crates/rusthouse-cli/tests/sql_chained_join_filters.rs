use std::fs;
use std::process::Command;

#[test]
fn chained_inner_join_literal_filters() {
    let db = std::env::temp_dir().join(format!("rusthouse-chain-filters-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE a (id UInt64, qty Nullable(UInt64))",
        "CREATE TABLE b (id UInt64, next UInt64, label String)",
        "CREATE TABLE c (id UInt64, price UInt64, day Date)",
        "INSERT INTO a VALUES (1, 2), (2, 5), (3, NULL)",
        "INSERT INTO b VALUES (1, 10, 'yes'), (1, 11, 'no'), (2, 10, 'yes'), (3, 10, 'yes')",
        "INSERT INTO c VALUES (10, 8, '1994-01-01'), (11, 9, '1995-01-01')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT COUNT(*) AS n FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE qty > 1 AND label = 'yes' AND price >= 8",
            "n\n2\n",
        ),
        (
            "SELECT a.id FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE price > 8 ORDER BY a.id",
            "a.id\n1\n",
        ),
        (
            "SELECT COUNT(qty) AS n FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE label = 'yes'",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE qty < 10 LIMIT 0",
            "n\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM a JOIN b ON a.id = b.id ANY INNER JOIN c ON b.next = c.id WHERE price > 8",
            "n\n1\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM a ANY INNER JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE label = 'no'",
            "n\n0\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE day >= DATE '1995-01-01'",
            "n\n1\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT COUNT(*) FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE missing = 1 LIMIT 0",
        "SELECT COUNT(*) FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE qty = 'bad' LIMIT 0",
        "SELECT COUNT(*) FROM a LEFT JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE price = 8",
        "SELECT a.id, COUNT(*) FROM a JOIN b ON a.id = b.id JOIN c ON b.next = c.id WHERE price = 8 GROUP BY a.id",
    ] {
        let result = sql(query);
        assert!(!result.status.success(), "{query}: {result:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
