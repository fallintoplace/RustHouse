use std::fs;
use std::process::Command;

#[test]
fn join_group_and_sort_resolve_unique_names() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-join-group-order-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lefts (id UInt64, category Nullable(String), age UInt64)",
        "CREATE TABLE rights (id UInt64, label String)",
        "CREATE TABLE tags (id UInt64, tag String)",
        "INSERT INTO lefts VALUES (1, 'A', 20), (2, 'A', 30), (3, 'B', 40), (4, NULL, 50)",
        "INSERT INTO rights VALUES (1, 'z'), (2, 'x'), (3, 'y'), (4, 'w')",
        "INSERT INTO tags VALUES (1, 'a'), (2, 'b'), (3, 'c'), (4, 'd')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let query = "SELECT category, COUNT(*) AS n FROM lefts l JOIN rights r ON l.id = r.id GROUP BY category ORDER BY category NULLS LAST";
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"category\tn\nA\t2\nB\t1\n\\N\t1\n");
    let query = "SELECT category, COUNT(*) AS n FROM lefts l JOIN rights r ON l.id = r.id JOIN tags t ON r.id = t.id GROUP BY category ORDER BY category NULLS LAST";
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"category\tn\nA\t2\nB\t1\n\\N\t1\n");
    let query = "SELECT category, COUNT(*) AS n FROM lefts l JOIN rights r ON l.id = r.id WHERE age >= 25 GROUP BY category ORDER BY category NULLS LAST";
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"category\tn\nA\t1\nB\t1\n\\N\t1\n");
    let query = "SELECT category, label, COUNT(*) AS n FROM lefts l JOIN rights r ON l.id = r.id GROUP BY category, label ORDER BY category NULLS LAST, label";
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"category\tlabel\tn\nA\tx\t1\nA\tz\t1\nB\ty\t1\n\\N\tw\t1\n"
    );
    let output =
        sql("SELECT label FROM lefts l JOIN rights r ON l.id = r.id ORDER BY label LIMIT 2");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"label\nw\nx\n");
    for query in [
        "SELECT label FROM lefts l JOIN rights r ON l.id = r.id ORDER BY id",
        "SELECT label FROM lefts l JOIN rights r ON l.id = r.id ORDER BY missing LIMIT 0",
        "SELECT id, COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id GROUP BY id",
        "SELECT category, COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE label = 'x' GROUP BY category",
        "SELECT category, COUNT(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE id = 1 GROUP BY category",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
