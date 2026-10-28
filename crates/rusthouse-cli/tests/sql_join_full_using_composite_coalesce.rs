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
fn full_using_composite_coalesce() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-full-using-composite-coalesce-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    successful(&db, "CREATE TABLE lefts (region UInt64, id UInt64, title String)");
    successful(&db, "CREATE TABLE rights (region UInt64, id UInt64, label String)");
    successful(&db, "INSERT INTO lefts VALUES (10, 1, 'L'), (20, 2, 'Left')");
    successful(&db, "INSERT INTO rights VALUES (10, 1, 'Match'), (30, 3, 'Right')");
    assert_eq!(successful(&db, "SELECT * FROM lefts l FULL JOIN rights r USING (id, region) ORDER BY l.id ASC NULLS LAST, r.id ASC NULLS LAST"), b"l.id\tl.region\tl.title\tr.label\n1\t10\tL\tMatch\n2\t20\tLeft\t\\N\n3\t30\t\\N\tRight\n");
    fs::remove_dir_all(db).unwrap();
}
