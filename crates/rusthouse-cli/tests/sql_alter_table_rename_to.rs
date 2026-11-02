use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn sql(db: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

fn successful(db: &Path, statement: &str) -> Output {
    let output = sql(db, statement);
    assert!(output.status.success(), "{statement}: {output:?}");
    output
}

#[test]
fn alter_rename_to_preserves_persistent_rows() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-alter_rename_to_preserves_persistent_rows-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    successful(&db, "CREATE TABLE events (id UInt64, label String)");
    successful(&db, "INSERT INTO events VALUES (1, 'a'), (2, 'b')");
    successful(&db, "ALTER TABLE events RENAME TO archive");
    assert_eq!(
        successful(&db, "SELECT id, label FROM archive ORDER BY id").stdout,
        b"id\\tlabel\\n1\\ta\\n2\\tb\\n"
    );
    assert!(!sql(&db, "SELECT id FROM events").status.success());
    assert!(!sql(&db, "ALTER TABLE archive RENAME TO archive").status.success());
    assert!(!sql(&db, "ALTER TABLE archive RENAME TO other; DROP TABLE archive").status.success());

    fs::remove_dir_all(db).unwrap();
}
