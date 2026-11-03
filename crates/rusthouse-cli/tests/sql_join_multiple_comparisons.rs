use std::fs;
use std::process::Command;

#[test]
fn joined_comparisons_filter_both_sides_and_keep_any_semantics() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-join-comparisons-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, qty Nullable(UInt64))",
        "CREATE TABLE dims (id UInt64, price Nullable(UInt64), label String)",
        "INSERT INTO facts VALUES (1, 10), (2, 20), (3, NULL), (4, 30)",
        "INSERT INTO dims VALUES (1, 50, 'first'), (1, 150, 'second'), (2, 120, 'third'), (3, 120, 'nullqty'), (4, NULL, 'nullprice')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let condition = "qty >= 10 AND qty <= 20 AND price >= 100 AND price < 200";
    let query = format!(
        "SELECT COUNT(*) AS matches FROM facts f JOIN dims d ON f.id = d.id WHERE {condition}"
    );
    let output = sql(&query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"matches\n2\n");
    let query = format!(
        "SELECT COUNT(*) AS matches FROM facts f ANY INNER JOIN dims d ON f.id = d.id WHERE {condition}"
    );
    let output = sql(&query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"matches\n1\n");
    let query = format!(
        "SELECT label FROM facts f JOIN dims d ON f.id = d.id WHERE {condition} LIMIT 1 OFFSET 1"
    );
    let output = sql(&query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"label\nthird\n");
    for query in [
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE qty >= 1 AND id = 1",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE qty >= 1 AND missing = 1 LIMIT 0",
        "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE qty >= 1 AND price IS NULL",
        "SELECT qty, COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE qty >= 1 AND price >= 1 GROUP BY qty",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    let empty = sql(
        "SELECT COUNT(*) AS matches FROM facts f JOIN dims d ON f.id = d.id WHERE qty >= 10 AND price >= 100 LIMIT 0",
    );
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"matches\n");
    let prefix = "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE ";
    let at_limit = format!("{}{}", prefix, vec!["qty >= 10"; 16].join(" AND "));
    assert!(sql(&at_limit).status.success());
    let over_limit = format!("{}{}", prefix, vec!["qty >= 10"; 17].join(" AND "));
    assert!(!sql(&over_limit).status.success());
    fs::remove_dir_all(db).unwrap();
}
