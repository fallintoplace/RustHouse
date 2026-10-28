use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn uuid_is_distinct_from_uses_binary_comparison_without_changing_sql_results() {
    let db = std::env::temp_dir().join(format!("rusthouse-uuid-distinct-from-{}", std::process::id()));
    for statement in [
        "CREATE TABLE events (id UInt64, uuid UUID)",
        "INSERT INTO events VALUES (1, '00000000-0000-0000-0000-000000000001'), (2, '00000000-0000-0000-0000-000000000002'), (3, 'ffffffff-ffff-ffff-ffff-ffffffffffff')",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        ("SELECT id FROM events WHERE uuid IS DISTINCT FROM '00000000-0000-0000-0000-000000000001' ORDER BY id", "id\n2\n3\n"),
        ("SELECT id FROM events WHERE uuid IS NOT DISTINCT FROM '00000000-0000-0000-0000-000000000001' ORDER BY id", "id\n1\n"),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
