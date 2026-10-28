use std::fs;
use std::process::Command;

#[test]
fn uuid_group_counts_use_exact_byte_keys_across_parts() {
    let db = std::env::temp_dir().join(format!("rusthouse-uuid-count-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (user_id UUID)",
        "INSERT INTO events VALUES ('00000000-0000-0000-0000-000000000001'), ('00000000-0000-0000-0000-000000000002')",
        "INSERT INTO events VALUES ('00000000-0000-0000-0000-000000000001'), ('00000000-0000-0000-0000-000000000001')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql("SELECT user_id, count(*) FROM events GROUP BY user_id ORDER BY user_id");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"user_id\tcount\n00000000-0000-0000-0000-000000000001\t3\n00000000-0000-0000-0000-000000000002\t1\n"
    );
    fs::remove_dir_all(db).unwrap();
}
