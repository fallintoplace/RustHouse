use std::fs;
use std::process::Command;

#[test]
fn uuid_grouped_multi_metrics_use_exact_uuid_keys_across_parts() {
    let db = std::env::temp_dir().join(format!("rusthouse-uuid-multi-{}", std::process::id()));
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
        "INSERT INTO events VALUES ('00000000-0000-0000-0000-000000000001', 7), ('00000000-0000-0000-0000-000000000002', 3)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT user_id, sum(amount), min(amount), max(amount) FROM events GROUP BY user_id ORDER BY user_id"
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"user_id\tsum(amount)\tmin(amount)\tmax(amount)\n00000000-0000-0000-0000-000000000001\t11\t4\t7\n00000000-0000-0000-0000-000000000002\t12\t3\t9\n"
    );
    fs::remove_dir_all(db).unwrap();
}
