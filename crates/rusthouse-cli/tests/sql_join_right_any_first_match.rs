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
fn right_any_first_match() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-right-any-first-match-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    successful(&db, "CREATE TABLE lefts (id UInt64, k UInt64)");
    successful(&db, "CREATE TABLE rights (id UInt64, k UInt64)");
    successful(&db, "INSERT INTO lefts VALUES (1, 10), (2, 10), (3, 20)");
    successful(
        &db,
        "INSERT INTO rights VALUES (50, 10), (60, 20), (70, 30)",
    );
    assert_eq!(
        successful(
            &db,
            "SELECT l.id, r.id FROM lefts l RIGHT ANY JOIN rights r ON l.k = r.k ORDER BY r.id"
        ),
        b"l.id\tr.id\n1\t50\n3\t60\n\\N\t70\n"
    );
    fs::remove_dir_all(db).unwrap();
}
