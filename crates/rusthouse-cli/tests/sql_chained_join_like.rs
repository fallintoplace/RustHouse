use std::fs;
use std::process::Command;

#[test]
fn chained_join_applies_like_before_order_and_aggregation() {
    let db = std::env::temp_dir().join(format!("rusthouse-chain-like-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE f (id UInt64, fk UInt64, score UInt64)",
        "CREATE TABLE d (dk UInt64, next UInt64, word Nullable(String))",
        "CREATE TABLE e (ek UInt64, label String)",
        "INSERT INTO f VALUES (1, 10, 2), (2, 20, 9), (3, 30, 3), (4, 40, 1)",
        "INSERT INTO d VALUES (10, 100, 'Alpha'), (10, 101, 'Alpine'), (20, 100, NULL), (30, 100, 'beta'), (40, 100, 'alpha')",
        "INSERT INTO e VALUES (100, 'z-one'), (101, 'z-two')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT f.id, e.label FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE word LIKE 'A%' AND score >= 2 ORDER BY f.id DESC, e.label",
            "f.id\te.label\n1\tz-one\n1\tz-two\n",
        ),
        (
            "SELECT f.id, e.label FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE word ILIKE 'a%' ORDER BY f.id DESC, e.label LIMIT 1 OFFSET 1",
            "f.id\te.label\n1\tz-one\n",
        ),
        (
            "SELECT count(*) AS n FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE word LIKE 'A%'",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(word) AS n FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE word NOT LIKE 'A%'",
            "n\n2\n",
        ),
        (
            "SELECT f.id, COUNT(*) AS n FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE word LIKE 'A%' GROUP BY f.id ORDER BY f.id",
            "f.id\tn\n1\t2\n",
        ),
        (
            "SELECT f.id, SUM(score) AS total FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE word LIKE 'A%' GROUP BY f.id ORDER BY f.id",
            "f.id\ttotal\n1\t4\n",
        ),
        (
            "SELECT f.id, e.label FROM f, d, e WHERE fk = dk AND next = ek AND word LIKE 'A%' ORDER BY f.id, e.label",
            "f.id\te.label\n1\tz-one\n1\tz-two\n",
        ),
        (
            "SELECT f.id, e.label FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE (word LIKE 'A%' OR score = 9) AND label LIKE 'z%' ORDER BY f.id, e.label",
            "f.id\te.label\n1\tz-one\n1\tz-two\n2\tz-one\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT f.id FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE missing LIKE 'A%' LIMIT 0",
        "SELECT f.id FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE score LIKE 'A%' LIMIT 0",
        "SELECT f.id FROM f LEFT JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE word LIKE 'A%'",
        "SELECT f.id FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE word LIKE 'A%' AND score NOT BETWEEN 1 AND 3",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
