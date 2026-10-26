use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-database-ddl-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn rename_preserves_stored_rows_and_drop_removes_both_files() {
    let db = database();
    assert!(sql("CREATE TABLE events (id UInt64, name String)", &db).status.success());
    assert!(sql("INSERT INTO events VALUES (1, 'first')", &db).status.success());

    let rename = sql("RENAME TABLE events TO archive", &db);
    assert!(rename.status.success(), "{rename:?}");
    assert!(!db.join("events.schema").exists());
    assert!(!db.join("events.parts").exists());
    assert!(db.join("archive.schema").is_file());
    assert!(db.join("archive.parts").is_dir());
    let read = sql("SELECT id, name FROM archive", &db);
    assert!(read.status.success(), "{read:?}");
    assert_eq!(read.stdout, b"id\tname\n1\tfirst\n");
    assert!(sql("INSERT INTO archive VALUES (2, 'second')", &db).status.success());
    let count = sql("SELECT count(*) FROM archive", &db);
    assert_eq!(count.stdout, b"count\n2\n");
    let gone = sql("SELECT * FROM events", &db);
    assert!(!gone.status.success());

    let drop = sql("DROP TABLE archive", &db);
    assert!(drop.status.success(), "{drop:?}");
    assert!(!db.join("archive.schema").exists());
    assert!(!db.join("archive.parts").exists());
    assert!(sql("DROP TABLE IF EXISTS archive", &db).status.success());
    assert!(!sql("DROP TABLE archive", &db).status.success());
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn conflicting_rename_does_not_modify_either_table() {
    let db = database();
    assert!(sql("CREATE TABLE a (id UInt64)", &db).status.success());
    assert!(sql("CREATE TABLE b (id UInt64)", &db).status.success());
    assert!(sql("INSERT INTO a VALUES (10)", &db).status.success());
    assert!(sql("INSERT INTO b VALUES (20)", &db).status.success());

    let conflict = sql("RENAME TABLE a TO b", &db);
    assert!(!conflict.status.success(), "{conflict:?}");
    assert_eq!(sql("SELECT id FROM a", &db).stdout, b"id\n10\n");
    assert_eq!(sql("SELECT id FROM b", &db).stdout, b"id\n20\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn drop_does_not_silently_ignore_orphaned_parts() {
    let db = database();
    assert!(sql("CREATE TABLE t (id UInt64)", &db).status.success());
    fs::remove_file(db.join("t.schema")).unwrap();
    let result = sql("DROP TABLE IF EXISTS t", &db);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("orphaned data"));
    assert!(db.join("t.parts").exists());
    fs::remove_dir_all(db).unwrap();
}
