use std::fs;
use std::process::Command;

#[test]
fn implicit_inner_join_matches_explicit_join() {
    let db = std::env::temp_dir().join(format!("rusthouse-implicit-join-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lefts (id UInt64, k Nullable(UInt64))",
        "CREATE TABLE rights (id UInt64, k Nullable(UInt64), name String)",
        "CREATE TABLE thirds (id UInt64, label String)",
        "INSERT INTO lefts VALUES (1, 10), (2, NULL), (3, 10)",
        "INSERT INTO rights VALUES (1, 10, 'one'), (3, 10, 'three'), (3, 10, 'again'), (2, NULL, 'null')",
        "INSERT INTO thirds VALUES (1, 'a'), (3, 'c')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (implicit, explicit) in [
        (
            "SELECT l.id, r.name FROM lefts l, rights r WHERE l.id = r.id AND l.k = r.k ORDER BY l.id",
            "SELECT l.id, r.name FROM lefts l JOIN rights r ON l.id = r.id AND l.k = r.k ORDER BY l.id",
        ),
        (
            "SELECT l.id, r.name, t.label FROM lefts l, rights r, thirds t WHERE t.id = r.id AND l.id = r.id ORDER BY l.id",
            "SELECT l.id, r.name, t.label FROM lefts l JOIN rights r ON l.id = r.id JOIN thirds t ON t.id = r.id ORDER BY l.id",
        ),
        (
            "SELECT count(*) FROM lefts l, rights r WHERE l.id = r.id AND l.id > 1",
            "SELECT count(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE l.id > 1",
        ),
    ] {
        let actual = sql(implicit);
        let expected = sql(explicit);
        assert!(actual.status.success(), "{implicit}: {actual:?}");
        assert!(expected.status.success(), "{explicit}: {expected:?}");
        assert_eq!(actual.stdout, expected.stdout, "{implicit}");
    }
    for query in [
        "SELECT l.id FROM lefts l, rights r WHERE l.id = r.id OR l.id = 1",
        "SELECT l.id FROM lefts l, rights r WHERE l.id = 1",
        "SELECT l.id FROM lefts l, rights r WHERE l.id = r.id AND r.id = 1 AND r.k = 10",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
