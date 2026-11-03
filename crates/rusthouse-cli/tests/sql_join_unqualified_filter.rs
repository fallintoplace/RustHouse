use std::fs;
use std::process::Command;

#[test]
fn join_filters_resolve_unique_names_and_reject_ambiguity() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-filter-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lefts (id UInt64, age Nullable(UInt64))",
        "CREATE TABLE rights (id UInt64, score Nullable(UInt64))",
        "CREATE TABLE tags (id UInt64, tag Nullable(String))",
        "INSERT INTO lefts VALUES (1, 20), (2, NULL), (3, 40), (4, 60)",
        "INSERT INTO rights VALUES (1, 10), (2, NULL), (3, 30)",
        "INSERT INTO tags VALUES (1, 'x'), (2, NULL)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE age >= 25",
            "count\n1\n",
        ),
        (
            "SELECT COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE score >= 20",
            "count\n1\n",
        ),
        (
            "SELECT COUNT(*) FROM lefts l LEFT JOIN rights r ON l.id = r.id WHERE score IS NULL",
            "count\n2\n",
        ),
        (
            "SELECT COUNT(*) FROM lefts l LEFT JOIN rights r ON l.id = r.id WHERE score IS NOT NULL",
            "count\n2\n",
        ),
        (
            "SELECT COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id JOIN tags t ON r.id = t.id WHERE tag IS NULL",
            "count\n1\n",
        ),
        (
            "SELECT COUNT(*) FROM lefts l, rights r WHERE l.id = r.id AND age >= 25",
            "count\n1\n",
        ),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    assert!(sql("INSERT INTO rights VALUES (1, 90)").status.success());
    let any = sql(
        "SELECT COUNT(*) FROM lefts l ANY INNER JOIN rights r ON l.id = r.id WHERE score >= 50",
    );
    assert!(any.status.success(), "{any:?}");
    assert_eq!(any.stdout, b"count\n0\n");
    let all =
        sql("SELECT COUNT(*) FROM lefts l INNER JOIN rights r ON l.id = r.id WHERE score >= 50");
    assert!(all.status.success(), "{all:?}");
    assert_eq!(all.stdout, b"count\n1\n");
    for query in [
        "SELECT COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE id = 1",
        "SELECT COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE missing = 1",
        "SELECT COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE id IS NULL",
        "SELECT COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE id = 1 LIMIT 0",
        "SELECT COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id JOIN tags t ON r.id = t.id WHERE id IS NULL LIMIT 0",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
