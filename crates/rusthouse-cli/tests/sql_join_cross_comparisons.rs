use std::fs;
use std::process::Command;

#[test]
fn cross_input_comparisons_filter_after_any_selection() {
    let db = std::env::temp_dir().join(format!("rusthouse-cross-compare-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, qty Nullable(UInt64), word String)",
        "CREATE TABLE dims (id UInt64, threshold Nullable(UInt64), word String)",
        "INSERT INTO facts VALUES (1, 5, 'a'), (2, 3, 'b'), (3, NULL, 'c'), (4, 9, 'd')",
        "INSERT INTO dims VALUES (1, 3, 'b'), (1, 8, 'a'), (2, 4, 'a'), (3, 9, 'c'), (4, NULL, 'd')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE f.qty < d.threshold",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE d.threshold > f.qty",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f ANY INNER JOIN dims d ON f.id = d.id WHERE f.qty < d.threshold",
            "n\n1\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE f.qty < d.threshold AND f.word = d.word",
            "n\n1\n",
        ),
        (
            "SELECT f.qty FROM facts f JOIN dims d ON f.id = d.id WHERE f.qty < d.threshold LIMIT 1",
            "f.qty\n5\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE f.qty < d.threshold LIMIT 0",
            "n\n",
        ),
        (
            "SELECT f.word, SUM(CASE WHEN threshold > 4 THEN 1 ELSE 0 END) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE f.qty < d.threshold GROUP BY f.word ORDER BY f.word",
            "f.word\tn\na\t1\nb\t0\n",
        ),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE f.qty < d.word",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE f.qty < d.missing LIMIT 0",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
