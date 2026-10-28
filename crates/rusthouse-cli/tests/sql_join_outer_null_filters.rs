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
fn outer_null_filters() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-outer-null-filters-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    successful(&db, "CREATE TABLE lefts (id UInt64)");
    successful(&db, "CREATE TABLE rights (id UInt64)");
    successful(&db, "INSERT INTO lefts VALUES (1), (2), (3)");
    successful(&db, "INSERT INTO rights VALUES (2), (4)");
    assert_eq!(
        successful(
            &db,
            "SELECT l.id FROM lefts l LEFT JOIN rights r ON l.id = r.id WHERE r.id IS NULL ORDER BY l.id"
        ),
        b"l.id\n1\n3\n"
    );
    assert_eq!(
        successful(
            &db,
            "SELECT l.id FROM lefts l LEFT JOIN rights r ON l.id = r.id WHERE r.id IS NOT NULL ORDER BY l.id"
        ),
        b"l.id\n2\n"
    );
    assert_eq!(
        successful(
            &db,
            "SELECT r.id FROM lefts l FULL JOIN rights r ON l.id = r.id WHERE l.id IS NULL ORDER BY r.id"
        ),
        b"r.id\n4\n"
    );
    fs::remove_dir_all(db).unwrap();
}
