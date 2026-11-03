use std::fs;
use std::process::Command;

#[test]
fn joined_column_comparisons_skip_nulls_and_keep_any_first_match() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-column-compare-{}",
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
        "CREATE TABLE facts (id UInt64, qty Nullable(UInt64), ceiling Nullable(UInt64))",
        "CREATE TABLE dims (id UInt64, start Nullable(Date), finish Nullable(Date), label String)",
        "INSERT INTO facts VALUES (1, 3, 5), (2, 6, 5), (3, NULL, 5)",
        "INSERT INTO dims VALUES (1, '1994-01-03', '1994-01-02', 'first'), (1, '1994-01-01', '1994-01-02', 'second'), (2, '1994-01-01', '1994-01-02', 'third'), (3, NULL, '1994-01-02', 'null')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE qty < ceiling AND start < finish",
            "n\n1\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f ANY INNER JOIN dims d ON f.id = d.id WHERE qty < ceiling AND start < finish",
            "n\n0\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE start < finish AND qty >= 1 AND finish < DATE '1995-01-01'",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM facts f JOIN dims d ON f.id = d.id WHERE start < finish LIMIT 0",
            "n\n",
        ),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE qty < price",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE qty < label",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE start < label",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE id < qty",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE qty < ceiling AND start IS NULL",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
