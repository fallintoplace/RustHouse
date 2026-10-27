use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn join_limit_accepts_clickhouse_offset_count_syntax() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-limit-comma-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lefts (id UInt64, k UInt64)",
        "CREATE TABLE rights (k UInt64, label String)",
        "INSERT INTO lefts VALUES (1, 10), (2, 10), (3, 20)",
        "INSERT INTO rights VALUES (10, 'a'), (10, 'b'), (20, 'c')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    let comma = sql("SELECT l.id, r.label FROM lefts l JOIN rights r ON l.k = r.k LIMIT 1, 2");
    assert!(comma.status.success(), "{comma:?}");
    assert_eq!(comma.stdout, b"l.id\tr.label\n1\tb\n2\ta\n");

    let explicit =
        sql("SELECT l.id, r.label FROM lefts l JOIN rights r ON l.k = r.k LIMIT 2 OFFSET 1");
    assert!(explicit.status.success(), "{explicit:?}");
    assert_eq!(explicit.stdout, comma.stdout);

    let zero = sql("SELECT l.id FROM lefts l JOIN rights r ON l.k = r.k LIMIT 3, 0");
    assert!(zero.status.success(), "{zero:?}");
    assert_eq!(zero.stdout, b"l.id\n");
    fs::remove_dir_all(db).unwrap();
}
