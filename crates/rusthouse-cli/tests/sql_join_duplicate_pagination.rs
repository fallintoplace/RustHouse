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
fn duplicate_pagination() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-duplicate-pagination-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    successful(&db, "CREATE TABLE lefts (id UInt64, k UInt64)");
    successful(&db, "CREATE TABLE rights (k UInt64, label String)");
    successful(&db, "INSERT INTO lefts VALUES (1, 5), (2, 5)");
    successful(&db, "INSERT INTO rights VALUES (5, 'a'), (5, 'b'), (5, 'c')");
    assert_eq!(successful(&db, "SELECT l.id, r.label FROM lefts l JOIN rights r ON l.k = r.k ORDER BY l.id, r.label LIMIT 2, 3"), b"l.id\tr.label\n1\tc\n2\ta\n2\tb\n");
    fs::remove_dir_all(db).unwrap();
}
