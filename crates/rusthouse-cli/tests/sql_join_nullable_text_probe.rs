use std::fs;
use std::process::Command;

#[test]
fn join_null_filter_on_borrowed_text_columns() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-text-null-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for query in [
        "CREATE TABLE people (id UInt64, label Nullable(String))",
        "CREATE TABLE events (id UInt64, person_id UInt64)",
        "INSERT INTO people VALUES (1, 'alpha'), (2, NULL), (3, 'gamma')",
        "INSERT INTO events VALUES (10, 1), (11, 2), (12, 3)",
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT p.id FROM people p JOIN events e ON p.id = e.person_id WHERE p.label IS NULL",
            "p.id\n2\n",
        ),
        (
            "SELECT p.id FROM people p JOIN events e ON p.id = e.person_id WHERE p.label IS NOT NULL",
            "p.id\n1\n3\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
