use std::fs;
use std::process::Command;

#[test]
fn join_resolves_unique_projection_names_and_rejects_ambiguous_names() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-join-unqualified-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lefts (id UInt64, value String)",
        "CREATE TABLE rights (id UInt64, label String)",
        "CREATE TABLE tags (id UInt64, tag String)",
        "INSERT INTO lefts VALUES (1, 'a'), (2, 'b')",
        "INSERT INTO rights VALUES (1, 'first')",
        "INSERT INTO tags VALUES (1, 'x')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let query = "SELECT value, label FROM lefts l LEFT JOIN rights r ON l.id = r.id ORDER BY l.id";
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"value\tlabel\na\tfirst\nb\t\\N\n");
    let query = "SELECT value, label, tag FROM lefts l JOIN rights r ON l.id = r.id JOIN tags t ON r.id = t.id ORDER BY l.id";
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"value\tlabel\ttag\na\tfirst\tx\n");
    for query in [
        "SELECT id FROM lefts l JOIN rights r ON l.id = r.id",
        "SELECT id FROM lefts l JOIN rights r ON l.id = r.id LIMIT 0",
        "SELECT missing FROM lefts l JOIN rights r ON l.id = r.id",
        "SELECT id FROM lefts l JOIN rights r ON l.id = r.id JOIN tags t ON r.id = t.id",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
