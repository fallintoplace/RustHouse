use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-create-publication-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(statement: &str, directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(directory)
        .output()
        .unwrap()
}

#[test]
fn creating_persistent_table_leaves_only_complete_public_metadata() {
    let db = database();
    let created = sql(
        "CREATE TABLE durable (id UInt64, note Nullable(String))",
        &db,
    );
    assert!(created.status.success(), "{created:?}");
    assert!(db.join("durable.schema").is_file());
    assert!(db.join("durable.parts/all_0_0_0/checksums.txt").is_file());
    assert!(fs::read_dir(&db).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("schema.tmp")
    }));
    assert_eq!(
        sql("SELECT count(*) FROM durable", &db).stdout,
        b"count\n0\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn orphaned_parts_from_interrupted_create_are_not_overwritten() {
    let db = database();
    fs::create_dir_all(db.join("incomplete.parts")).unwrap();
    let show = sql("SHOW TABLES", &db);
    assert!(show.status.success(), "{show:?}");
    assert_eq!(show.stdout, b"name\n");

    let create = sql("CREATE TABLE incomplete (id UInt64)", &db);
    assert!(!create.status.success(), "{create:?}");
    assert!(db.join("incomplete.parts").is_dir());
    assert!(!db.join("incomplete.schema").exists());
    fs::remove_dir_all(db).unwrap();
}
