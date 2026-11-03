use std::fs;
use std::process::Command;

#[test]
fn joined_in_filters_keep_null_and_any_semantics() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-in-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, qty Nullable(UInt64))",
        "CREATE TABLE dims (id UInt64, mode Nullable(String), start Date, finish Date)",
        "INSERT INTO facts VALUES (1, 5), (2, 10), (3, NULL)",
        "INSERT INTO dims VALUES (1, 'ROAD', '1994-01-01', '1994-01-02'), (1, 'SHIP', '1994-01-01', '1994-01-02'), (2, 'MAIL', '1994-01-01', '1994-01-02'), (3, NULL, '1994-01-01', '1994-01-02')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE mode IN ('MAIL','SHIP','SHIP') AND start < finish AND qty >= 1",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f ANY INNER JOIN dims d ON f.id = d.id WHERE mode IN ('MAIL','SHIP') AND start < finish",
            "n\n1\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE qty IN (5,10) AND mode IN ('SHIP','MAIL')",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE mode IN ('AIR')",
            "n\n0\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE mode IN ('SHIP') LIMIT 0",
            "n\n",
        ),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE id IN (1)",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE qty IN ('bad')",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE mode IN (NULL)",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE mode IN ()",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE mode NOT IN ('SHIP')",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    let prefix = "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE qty IN (";
    let at_limit = format!("{}{})", prefix, vec!["5"; 64].join(","));
    assert!(sql(&at_limit).status.success());
    let over_limit = format!("{}{})", prefix, vec!["5"; 65].join(","));
    assert!(!sql(&over_limit).status.success());
    fs::remove_dir_all(db).unwrap();
}
