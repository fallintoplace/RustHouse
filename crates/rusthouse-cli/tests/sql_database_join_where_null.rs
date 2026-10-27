use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn join_filters_null_extended_rows_and_streamed_matches() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-where-null-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lefts (id UInt64, k Nullable(UInt64))",
        "CREATE TABLE rights (id UInt64, k Nullable(UInt64))",
        "INSERT INTO lefts VALUES (1, 10), (2, 20), (3, NULL)",
        "INSERT INTO rights VALUES (7, 10), (8, 30), (9, NULL)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    let anti_left = sql(
        "SELECT l.id FROM lefts l LEFT JOIN rights r ON l.k = r.k WHERE r.id IS NULL ORDER BY l.id LIMIT 10",
    );
    assert!(anti_left.status.success(), "{anti_left:?}");
    assert_eq!(anti_left.stdout, b"l.id\n2\n3\n");

    let anti_right = sql(
        "SELECT r.id FROM lefts l FULL JOIN rights r ON l.k = r.k WHERE l.id IS NULL ORDER BY r.id LIMIT 10",
    );
    assert!(anti_right.status.success(), "{anti_right:?}");
    assert_eq!(anti_right.stdout, b"r.id\n8\n9\n");

    let matched = sql("SELECT l.id FROM lefts l JOIN rights r ON l.k = r.k WHERE r.k IS NOT NULL");
    assert!(matched.status.success(), "{matched:?}");
    assert_eq!(matched.stdout, b"l.id\n1\n");

    let missing = sql("SELECT l.id FROM lefts l JOIN rights r ON l.k = r.k WHERE r.id IS NULL");
    assert!(missing.status.success(), "{missing:?}");
    assert_eq!(missing.stdout, b"l.id\n");
    let invalid = sql("SELECT l.id FROM lefts l JOIN rights r ON l.k = r.k WHERE x.k IS NULL");
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unknown JOIN qualifier"));
    fs::remove_dir_all(db).unwrap();
}
