use std::fs;
use std::process::Command;

#[test]
fn chained_join_count_column_skips_nulls_and_preserves_matches() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-chain-count-column-{}",
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
        "CREATE TABLE facts (id UInt64, amount Nullable(Int64))",
        "CREATE TABLE dims (id UInt64, label Nullable(String))",
        "CREATE TABLE tags (id UInt64, tag Nullable(String))",
        "INSERT INTO facts VALUES (1, 10), (2, NULL), (3, -5)",
        "INSERT INTO dims VALUES (1, 'a'), (1, NULL), (2, 'b'), (3, NULL)",
        "INSERT INTO tags VALUES (1, 'x'), (2, NULL), (3, 'z')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT COUNT(amount) FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id",
            "count(amount)\n3\n",
        ),
        (
            "SELECT COUNT(label) AS n FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id",
            "n\n2\n",
        ),
        (
            "SELECT COUNT(t.tag) FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id",
            "count(t.tag)\n3\n",
        ),
        (
            "SELECT COUNT(amount) FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id WHERE label IS NULL",
            "count(amount)\n2\n",
        ),
        (
            "SELECT COUNT(amount) FROM facts f ANY INNER JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id",
            "count(amount)\n2\n",
        ),
        (
            "SELECT COUNT(label) FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id WHERE tag IS NULL",
            "count(label)\n1\n",
        ),
        (
            "SELECT COUNT(amount) FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id LIMIT 0",
            "count(amount)\n",
        ),
        (
            "SELECT COUNT(amount) FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id OFFSET 1",
            "count(amount)\n",
        ),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    assert!(
        sql("CREATE TABLE empty_facts (id UInt64, empty_amount Nullable(Int64))")
            .status
            .success()
    );
    let empty = sql(
        "SELECT COUNT(empty_amount) FROM empty_facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id",
    );
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"count(empty_amount)\n0\n");
    for query in [
        "SELECT COUNT(id) FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id",
        "SELECT COUNT(id) FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id LIMIT 0",
        "SELECT COUNT(missing) FROM facts f JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id",
        "SELECT COUNT(amount) FROM facts f LEFT JOIN dims d ON f.id = d.id JOIN tags t ON d.id = t.id",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
