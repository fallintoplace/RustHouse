use std::fs;
use std::process::Command;

#[test]
fn join_aggregate_resolves_unique_columns_and_skips_nulls() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-aggregate-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, qty Nullable(Int64))",
        "CREATE TABLE dims (id UInt64, price Nullable(Int64))",
        "INSERT INTO facts VALUES (1, 10), (2, NULL), (3, -5)",
        "INSERT INTO dims VALUES (1, 5), (1, 7), (2, NULL), (3, 9)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (expression, expected) in [
        ("COUNT(qty)", "count(qty)\n3\n"),
        ("COUNT(price)", "count(price)\n3\n"),
        ("SUM(qty)", "sum(qty)\n15\n"),
        ("SUM(price)", "sum(price)\n21\n"),
        ("AVG(qty)", "avg(qty)\n5\n"),
        ("MIN(qty)", "min(qty)\n-5\n"),
        ("MAX(qty)", "max(qty)\n10\n"),
        ("SUM(qty) AS total", "total\n15\n"),
    ] {
        let query = format!("SELECT {expression} FROM facts f JOIN dims d ON f.id = d.id");
        let output = sql(&query);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    for expression in ["SUM(id)", "COUNT(id)", "SUM(missing)", "SUM(id) LIMIT 0"] {
        let query = if expression.ends_with("LIMIT 0") {
            "SELECT SUM(id) FROM facts f JOIN dims d ON f.id = d.id LIMIT 0".to_owned()
        } else {
            format!("SELECT {expression} FROM facts f JOIN dims d ON f.id = d.id")
        };
        let output = sql(&query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
