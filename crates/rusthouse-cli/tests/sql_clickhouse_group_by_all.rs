use std::fs;
use std::process::Command;

#[test]
fn clickhouse_group_by_all_deduplicates_rows_and_groups_metrics() {
    let path = std::env::temp_dir().join(format!("rusthouse-group-all-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&path)
            .output()
            .unwrap()
    };
    let created = sql("CREATE TABLE events (kind String, amount UInt64)");
    assert!(created.status.success(), "{created:?}");
    let inserted = sql("INSERT INTO events VALUES ('x', 1), ('x', 3), ('y', 5)");
    assert!(inserted.status.success(), "{inserted:?}");
    let distinct = sql("SELECT kind FROM events GROUP BY ALL ORDER BY kind");
    assert!(distinct.status.success(), "{distinct:?}");
    assert_eq!(distinct.stdout, b"kind\nx\ny\n");
    let grouped = sql("SELECT kind, count(*) FROM events GROUP BY ALL ORDER BY kind");
    assert!(grouped.status.success(), "{grouped:?}");
    assert_eq!(grouped.stdout, b"kind\tcount\nx\t2\ny\t1\n");
    fs::remove_dir_all(path).unwrap();
}
