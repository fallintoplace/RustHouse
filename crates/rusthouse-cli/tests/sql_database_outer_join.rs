use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-outer-join-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn left_right_and_full_joins_null_extend_unmatched_rows() {
    let db = database();
    for statement in [
        "CREATE TABLE lefts (id UInt64, k Nullable(UInt64))",
        "CREATE TABLE rights (id UInt64, k Nullable(UInt64))",
        "INSERT INTO lefts VALUES (1, 10), (2, 20), (3, NULL)",
        "INSERT INTO rights VALUES (7, 10), (8, 30), (9, NULL)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let left = sql(
        "SELECT l.id, r.id FROM lefts l LEFT JOIN rights r ON l.k = r.k ORDER BY l.id LIMIT 10",
        &db,
    );
    assert!(left.status.success(), "{left:?}");
    assert_eq!(left.stdout, b"l.id\tr.id\n1\t7\n2\t\\N\n3\t\\N\n");

    let right = sql(
        "SELECT l.id, r.id FROM lefts l RIGHT OUTER JOIN rights r ON l.k = r.k ORDER BY r.id LIMIT 10",
        &db,
    );
    assert!(right.status.success(), "{right:?}");
    assert_eq!(right.stdout, b"l.id\tr.id\n1\t7\n\\N\t8\n\\N\t9\n");

    let full = sql(
        "SELECT l.id, r.id FROM lefts l FULL OUTER JOIN rights r ON l.k = r.k ORDER BY l.id LIMIT 10",
        &db,
    );
    assert!(full.status.success(), "{full:?}");
    assert_eq!(
        full.stdout,
        b"l.id\tr.id\n\\N\t8\n\\N\t9\n1\t7\n2\t\\N\n3\t\\N\n"
    );
    fs::remove_dir_all(db).unwrap();
}
