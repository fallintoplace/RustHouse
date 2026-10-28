use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(db: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

fn successful(db: &Path, statement: &str) -> Vec<u8> {
    let output = sql(db, statement);
    assert!(
        output.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn composite_null_keys() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-composite-null-keys-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    successful(&db, "CREATE TABLE lefts (id UInt64, k Nullable(UInt64))");
    successful(&db, "CREATE TABLE rights (id UInt64, k Nullable(UInt64), label String)");
    successful(&db, "INSERT INTO lefts VALUES (1, 10), (2, NULL), (3, 20)");
    successful(&db, "INSERT INTO rights VALUES (1, 10, 'hit'), (2, NULL, 'null'), (3, 21, 'wrong')");
    assert_eq!(successful(&db, "SELECT l.id, r.label FROM lefts l JOIN rights r ON l.id = r.id AND l.k = r.k ORDER BY l.id"), b"l.id\tr.label\n1\thit\n");
    assert_eq!(successful(&db, "SELECT l.id, r.label FROM lefts l LEFT JOIN rights r ON l.id = r.id AND l.k = r.k ORDER BY l.id"), b"l.id\tr.label\n1\thit\n2\t\\N\n3\t\\N\n");
    fs::remove_dir_all(db).unwrap();
}
