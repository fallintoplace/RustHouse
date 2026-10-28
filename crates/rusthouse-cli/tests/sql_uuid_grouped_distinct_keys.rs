use std::fs;
use std::process::Command;

#[test]
fn group_uuid_keys_are_exact_when_aggregating_distinct_text() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-uuid-distinct-group-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (user_id UUID, session_id String)",
        "INSERT INTO events VALUES ('00000000-0000-0000-0000-000000000001', 'a'), ('00000000-0000-0000-0000-000000000002', 'a')",
        "INSERT INTO events VALUES ('00000000-0000-0000-0000-000000000001', 'b'), ('00000000-0000-0000-0000-000000000002', 'a')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT user_id, count(DISTINCT session_id) FROM events GROUP BY user_id ORDER BY user_id"
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"user_id\tcountDistinct(session_id)\n00000000-0000-0000-0000-000000000001\t2\n00000000-0000-0000-0000-000000000002\t1\n"
    );
    fs::remove_dir_all(db).unwrap();
}
