use std::fs;
use std::process::Command;

#[test]
fn chained_right_any_keeps_only_first_left_match_per_right_row() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-chain-right-any-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE a (id UInt64, k UInt64)",
        "CREATE TABLE b (id UInt64, k UInt64)",
        "CREATE TABLE c (id UInt64, k UInt64)",
        "INSERT INTO a VALUES (1, 10), (2, 10)",
        "INSERT INTO b VALUES (7, 10), (8, 10)",
        "INSERT INTO c VALUES (99, 10), (100, 20)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT x.id, y.id, z.id FROM a x RIGHT ANY JOIN b y ON x.k = y.k RIGHT ANY JOIN c z ON y.k = z.k ORDER BY z.id";
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"x.id\ty.id\tz.id\n1\t7\t99\n\\N\t\\N\t100\n");
    fs::remove_dir_all(db).unwrap();
}
