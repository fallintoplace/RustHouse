use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn join_projects_qualified_wildcards_in_requested_order() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-stars-{}-{}",
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
        "CREATE TABLE lefts (id UInt64, title String)",
        "CREATE TABLE rights (id UInt64, label String)",
        "INSERT INTO lefts VALUES (1, 'first'), (2, 'second')",
        "INSERT INTO rights VALUES (2, 'match'), (3, 'third')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql("SELECT l.*, r.label FROM lefts l JOIN rights r ON l.id = r.id");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"l.id\tl.title\tr.label\n2\tsecond\tmatch\n");

    let result = sql("SELECT r.*, l.title FROM lefts l JOIN rights r USING (id)");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"r.id\tr.label\tl.title\n2\tmatch\tsecond\n");

    let invalid = sql("SELECT x.* FROM lefts l JOIN rights r ON l.id = r.id");
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unknown JOIN qualifier"));
    fs::remove_dir_all(db).unwrap();
}
