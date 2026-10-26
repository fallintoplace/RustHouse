use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-insert-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    dir
}

fn sql(query: &str, catalog: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--catalog"])
        .arg(catalog)
        .output()
        .unwrap()
}

fn part_sql(query: &str, table: &str, catalog: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--part"])
        .arg(catalog.join(format!("{table}.parts")))
        .output()
        .unwrap()
}

#[test]
fn insert_values_persist_across_commands_and_reorder_columns() {
    let root = catalog();
    assert!(
        sql(
            "CREATE TABLE events (id Int64, name String, score Nullable(UInt64))",
            &root
        )
        .status
        .success()
    );
    for statement in [
        "INSERT INTO events VALUES (-2, 'alice', 5), (7, 'bob', NULL)",
        "INSERT INTO events (name, id) VALUES ('carol', 8)",
    ] {
        let output = sql(statement, &root);
        assert!(output.status.success(), "{output:?}");
    }
    let count = part_sql("SELECT count(*) FROM part", "events", &root);
    assert!(count.status.success(), "{count:?}");
    assert_eq!(count.stdout, b"count\n3\n");
    let names = part_sql("SELECT name FROM part ORDER BY name", "events", &root);
    assert!(names.status.success(), "{names:?}");
    assert_eq!(names.stdout, b"name\nalice\nbob\ncarol\n");

    let invalid = sql("INSERT INTO events (name) VALUES ('missing id')", &root);
    assert!(!invalid.status.success());
    let invalid = sql(
        "INSERT INTO events VALUES (9, 'overflow', 18446744073709551616)",
        &root,
    );
    assert!(!invalid.status.success());
    let count = part_sql("SELECT count(*) FROM part", "events", &root);
    assert_eq!(count.stdout, b"count\n3\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn drop_and_rename_keep_table_data_consistent() {
    let root = catalog();
    assert!(sql("CREATE TABLE logs (id UInt8)", &root).status.success());
    assert!(sql("INSERT INTO logs VALUES (12)", &root).status.success());
    let rename = sql("RENAME TABLE logs TO archive", &root);
    assert!(rename.status.success(), "{rename:?}");
    assert!(!root.join("logs.parts").exists());
    assert!(root.join("archive.parts").is_dir());
    let count = part_sql("SELECT count(*) FROM part", "archive", &root);
    assert_eq!(count.stdout, b"count\n1\n");
    let alter = sql("ALTER TABLE archive ADD COLUMN message String", &root);
    assert!(!alter.status.success(), "{alter:?}");
    let drop = sql("DROP TABLE archive", &root);
    assert!(drop.status.success(), "{drop:?}");
    assert!(!root.join("archive.parts").exists());
    assert!(
        sql("CREATE TABLE archive (id UInt8)", &root)
            .status
            .success()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn insert_failure_does_not_publish_partial_part() {
    let root = catalog();
    assert!(
        sql("CREATE TABLE tiny (id UInt8, label String)", &root)
            .status
            .success()
    );
    let error = sql("INSERT INTO tiny VALUES (255, 'good'), (256, 'bad')", &root);
    assert!(!error.status.success(), "{error:?}");
    let parts = root.join("tiny.parts");
    if parts.exists() {
        assert_eq!(fs::read_dir(parts).unwrap().count(), 0);
    }
    assert!(
        sql("INSERT INTO tiny (label, id) VALUES ('ok', 1)", &root)
            .status
            .success()
    );
    let count = part_sql("SELECT count(*) FROM part", "tiny", &root);
    assert_eq!(count.stdout, b"count\n1\n");
    fs::remove_dir_all(root).unwrap();
}
