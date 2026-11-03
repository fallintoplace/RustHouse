use std::fs;
use std::process::Command;

#[test]
fn joined_like_filters_preserve_nulls_and_any_match_order() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-like-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, name Nullable(String))",
        "CREATE TABLE dims (id UInt64, label Nullable(String))",
        "INSERT INTO facts VALUES (1, 'Alpha'), (2, 'alpha'), (3, NULL), (4, 'A_B')",
        "INSERT INTO dims VALUES (1, 'bad'), (1, 'BRASS'), (2, 'brass'), (3, NULL), (4, 'BRASS')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE label LIKE '%BRASS'",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE label ILIKE '%brass'",
            "n\n3\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE label NOT LIKE '%BRASS'",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f ANY INNER JOIN dims d ON f.id = d.id WHERE label LIKE '%BRASS'",
            "n\n1\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE label ILIKE 'br%' AND name ILIKE 'a%'",
            "n\n3\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE name LIKE 'A\\\\_B'",
            "n\n1\n",
        ),
        (
            "SELECT name, SUM(CASE WHEN label = 'BRASS' THEN 1 ELSE 0 END) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE name ILIKE 'a%' GROUP BY name ORDER BY name",
            "name\tn\nA_B\t1\nAlpha\t1\nalpha\t0\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE label LIKE 'missing%' LIMIT 0",
            "n\n",
        ),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE id LIKE '1'",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE f.id LIKE '1' LIMIT 0",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE missing LIKE 'x'",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE name NOT IN ('x')",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
