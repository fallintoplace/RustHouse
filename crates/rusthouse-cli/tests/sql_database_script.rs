use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-script-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn run_script(file: &Path, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "--script"])
        .arg(file)
        .arg("--database")
        .arg(db)
        .output()
        .unwrap()
}

fn query(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn script_creates_inserts_and_queries_persistent_data() {
    let root = root();
    fs::create_dir(&root).unwrap();
    let db = root.join("db");
    let file = root.join("setup.sql");
    fs::write(
        &file,
        "-- create test table\nCREATE TABLE events (id UInt64, name String);\n\
         INSERT INTO events VALUES (1, 'a;b'), (2, 'Grace'); -- comment\n\
         SELECT name FROM events ORDER BY name;\n",
    )
    .unwrap();
    let result = run_script(&file, &db);
    assert!(result.status.success(), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stdout).contains("a;b"));
    assert_eq!(
        query("SELECT count(*) FROM events", &db).stdout,
        b"count\n2\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn script_stops_before_later_mutations_after_an_error() {
    let root = root();
    fs::create_dir(&root).unwrap();
    let db = root.join("db");
    let file = root.join("bad.sql");
    fs::write(
        &file,
        "CREATE TABLE events (id UInt8);\n\
         INSERT INTO events VALUES (1);\n\
         INSERT INTO events VALUES (256);\n\
         INSERT INTO events VALUES (2);\n",
    )
    .unwrap();
    let result = run_script(&file, &db);
    assert!(!result.status.success(), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stderr).contains("statement 3 failed"));
    assert_eq!(query("SELECT id FROM events", &db).stdout, b"id\n1\n");
    fs::remove_dir_all(root).unwrap();
}
