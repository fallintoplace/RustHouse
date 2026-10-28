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
fn composite_reversed_predicates() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-composite-reversed-predicates-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    successful(&db, "CREATE TABLE facts (id UInt64, team UInt64)");
    successful(&db, "CREATE TABLE labels (id UInt64, team UInt64, label String)");
    successful(&db, "INSERT INTO facts VALUES (1, 10), (2, 20), (3, 30)");
    successful(&db, "INSERT INTO labels VALUES (10, 1, 'A'), (30, 3, 'C'), (20, 4, 'wrong')");
    assert_eq!(successful(&db, "SELECT f.id, l.label FROM facts f JOIN labels l ON l.id = f.team AND f.id = l.team ORDER BY f.id"), b"f.id\tl.label\n1\tA\n3\tC\n");
    fs::remove_dir_all(db).unwrap();
}
