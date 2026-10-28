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
fn empty_outer_inputs() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-empty-outer-inputs-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    successful(&db, "CREATE TABLE lefts (id UInt64)");
    successful(&db, "CREATE TABLE rights (id UInt64)");
    assert_eq!(successful(&db, "SELECT * FROM lefts l FULL JOIN rights r USING (id) LIMIT 5"), b"l.id\n");
    successful(&db, "INSERT INTO lefts VALUES (1)");
    assert_eq!(successful(&db, "SELECT * FROM lefts l FULL JOIN rights r USING (id) LIMIT 5"), b"l.id\n1\n");
    successful(&db, "INSERT INTO rights VALUES (2)");
    assert_eq!(successful(&db, "SELECT * FROM lefts l FULL JOIN rights r USING (id) ORDER BY l.id NULLS LAST"), b"l.id\n1\n2\n");
    fs::remove_dir_all(db).unwrap();
}
