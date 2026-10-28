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
fn case_sensitive_text_keys() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-case-sensitive-text-keys-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    successful(&db, "CREATE TABLE accounts (id UInt64, email String)");
    successful(&db, "CREATE TABLE emails (email String, code String)");
    successful(
        &db,
        "INSERT INTO accounts VALUES (1, 'abc'), (2, 'Abc'), (3, 'abc')",
    );
    successful(
        &db,
        "INSERT INTO emails VALUES ('abc', 'lower'), ('Abc', 'upper')",
    );
    assert_eq!(
        successful(
            &db,
            "SELECT a.id, e.code FROM accounts a JOIN emails e ON a.email = e.email ORDER BY a.id"
        ),
        b"a.id\te.code\n1\tlower\n2\tupper\n3\tlower\n"
    );
    fs::remove_dir_all(db).unwrap();
}
