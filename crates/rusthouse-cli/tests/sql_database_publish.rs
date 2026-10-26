use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-atomic-insert-{}-{}",
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
fn only_published_part_directories_are_visible_to_select() {
    let db = database();
    assert!(sql("CREATE TABLE items (id UInt8)", &db).status.success());
    assert!(sql("INSERT INTO items VALUES (1), (2)", &db).status.success());
    let parts = db.join("items.parts");

    // Emulate an interrupted INSERT from the previous staging layout.
    let orphan = parts.join(".pending-interrupted");
    fs::create_dir(&orphan).unwrap();
    fs::write(orphan.join("count.txt"), "2\n").unwrap();
    fs::write(
        orphan.join("columns.txt"),
        "columns format version: 1\n1 columns:\n`id` UInt8\n",
    )
    .unwrap();
    let select = sql("SELECT count(*) FROM items", &db);
    assert!(select.status.success(), "{select:?}");
    assert_eq!(select.stdout, b"count\n2\n");

    let next = sql("INSERT INTO items VALUES (3)", &db);
    assert!(next.status.success(), "{next:?}");
    assert_eq!(sql("SELECT count(*) FROM items", &db).stdout, b"count\n3\n");
    assert!(fs::read_dir(&db).unwrap().all(|entry| {
        !entry.unwrap().file_name().to_string_lossy().contains(".pending-")
    }));
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn rejected_rows_cannot_publish_new_parts() {
    let db = database();
    assert!(sql("CREATE TABLE items (id UInt8)", &db).status.success());
    let bad = sql("INSERT INTO items VALUES (1), (256)", &db);
    assert!(!bad.status.success());
    assert_eq!(sql("SELECT count(*) FROM items", &db).stdout, b"count\n0\n");
    assert_eq!(fs::read_dir(db.join("items.parts")).unwrap().count(), 1);
    assert!(fs::read_dir(&db).unwrap().all(|entry| {
        !entry.unwrap().file_name().to_string_lossy().contains(".pending-")
    }));
    fs::remove_dir_all(db).unwrap();
}
