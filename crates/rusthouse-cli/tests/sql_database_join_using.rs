use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-using-{}-{}",
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
fn using_joins_deduplicate_wildcard_keys_and_preserve_outer_values() {
    let db = database();
    for statement in [
        "CREATE TABLE lefts (id UInt64, title String)",
        "CREATE TABLE rights (id UInt64, label String)",
        "INSERT INTO lefts VALUES (1, 'left-only'), (2, 'matched')",
        "INSERT INTO rights VALUES (2, 'right-match'), (3, 'right-only')",
    ] {
        let output = sql(statement, &db);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let output = sql(
        "SELECT * FROM lefts l FULL JOIN rights r USING (id) LIMIT 10",
        &db,
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"l.id\tl.title\tr.label\n1\tleft-only\t\\N\n2\tmatched\tright-match\n3\t\\N\tright-only\n"
    );
    let explicit = sql(
        "SELECT l.id, r.label FROM lefts l INNER JOIN rights r USING (id) LIMIT 10",
        &db,
    );
    assert!(explicit.status.success(), "{explicit:?}");
    assert_eq!(explicit.stdout, b"l.id\tr.label\n2\tright-match\n");
    let invalid = sql(
        "SELECT * FROM lefts l JOIN rights r USING (missing)",
        &db,
    );
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unknown JOIN column"));
    fs::remove_dir_all(db).unwrap();
}
