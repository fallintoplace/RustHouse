use std::fs;
use std::process::Command;

#[test]
fn all_join_preserves_duplicate_right_matches() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-all-join-{}",
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
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimensions (id UInt64, label String)",
        "INSERT INTO facts VALUES (7)",
        "INSERT INTO dimensions VALUES (7, 'a'), (7, 'b')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for query in [
        "SELECT f.id, d.label FROM facts f ALL INNER JOIN dimensions d ON f.id = d.id",
        "SELECT f.id, d.label FROM facts f INNER ALL JOIN dimensions d ON f.id = d.id",
        "SELECT f.id, d.label FROM facts f LEFT ALL JOIN dimensions d ON f.id = d.id",
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, b"f.id\td.label\n7\ta\n7\tb\n");
    }
    fs::remove_dir_all(db).unwrap();
}
