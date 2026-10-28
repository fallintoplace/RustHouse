use std::fs;
use std::process::Command;

#[test]
fn multiple_aggregates_distinct_uuid_uses_exact_byte_identity() {
    let db = std::env::temp_dir().join(format!("rusthouse-multi-uuid-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (user_id UUID, amount UInt64)",
        "INSERT INTO events VALUES ('00000000-0000-0000-0000-000000000001', 4), ('00000000-0000-0000-0000-000000000002', 9)",
        "INSERT INTO events VALUES ('00000000-0000-0000-0000-000000000001', 7), ('00000000-0000-0000-0000-000000000003', 3)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql("SELECT count(DISTINCT user_id), sum(amount) FROM events");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"countDistinct(user_id)\tsum(amount)\n3\t23\n");
    fs::remove_dir_all(db).unwrap();
}
