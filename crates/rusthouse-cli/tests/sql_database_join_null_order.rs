use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn join_order_by_respects_explicit_null_placement() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-nulls-order-{}-{}",
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
        "CREATE TABLE lefts (id UInt64, k Nullable(UInt64))",
        "CREATE TABLE rights (k Nullable(UInt64), label String)",
        "INSERT INTO lefts VALUES (1, 10), (2, NULL), (3, 20)",
        "INSERT INTO rights VALUES (10, 'a'), (20, 'b')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let first = sql(
        "SELECT l.id, r.label FROM lefts l LEFT JOIN rights r ON l.k = r.k ORDER BY r.label DESC NULLS FIRST LIMIT 10",
    );
    assert!(first.status.success(), "{first:?}");
    assert_eq!(first.stdout, b"l.id\tr.label\n2\t\\N\n3\tb\n1\ta\n");

    let last = sql(
        "SELECT l.id, r.label FROM lefts l LEFT JOIN rights r ON l.k = r.k ORDER BY r.label ASC NULLS LAST LIMIT 10",
    );
    assert!(last.status.success(), "{last:?}");
    assert_eq!(last.stdout, b"l.id\tr.label\n1\ta\n3\tb\n2\t\\N\n");

    let default =
        sql("SELECT l.id FROM lefts l LEFT JOIN rights r ON l.k = r.k ORDER BY r.label LIMIT 10");
    assert!(default.status.success(), "{default:?}");
    assert_eq!(default.stdout, b"l.id\n2\n1\n3\n");
    fs::remove_dir_all(db).unwrap();
}
