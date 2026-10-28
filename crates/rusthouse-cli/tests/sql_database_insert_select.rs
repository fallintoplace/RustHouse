use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-insert-select-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

fn ok(statement: &str, db: &Path) -> String {
    let result = sql(statement, db);
    assert!(
        result.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn copies_filtered_rows_between_persistent_tables() {
    let db = database();
    ok(
        "CREATE TABLE source (id UInt64, name String, note Nullable(String))",
        &db,
    );
    ok(
        "CREATE TABLE dest (id UInt64, name String, note Nullable(String))",
        &db,
    );
    ok(
        "INSERT INTO source VALUES (1, 'Ada', NULL), (2, 'Line\nbreak', 'a\\b'), (3, '\\N', NULL)",
        &db,
    );
    ok(
        "INSERT INTO dest SELECT id, name, note FROM source WHERE id >= 2 ORDER BY id",
        &db,
    );
    assert_eq!(
        ok("SELECT id, name, note FROM dest ORDER BY id", &db),
        "id\tname\tnote\n2\tLine\\nbreak\ta\\\\b\n3\t\\\\N\t\\N\n"
    );
    let error = sql("INSERT INTO dest (id, id) SELECT id FROM source", &db);
    assert!(!error.status.success());
    assert_eq!(ok("SELECT count(*) FROM dest", &db), "count\n2\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn generated_rows_and_empty_inserts_use_existing_publish_path() {
    let db = database();
    ok(
        "CREATE TABLE target (id UInt64, extra Nullable(String))",
        &db,
    );
    ok(
        "INSERT INTO target (id) SELECT number FROM generate_series(1, 5) WHERE number > 2",
        &db,
    );
    assert_eq!(
        ok("SELECT id FROM target ORDER BY id", &db),
        "id\n3\n4\n5\n"
    );
    ok("INSERT INTO target (id) SELECT number FROM numbers(0)", &db);
    assert_eq!(ok("SELECT count(*) FROM target", &db), "count\n3\n");
    // Bulk INSERT SELECT publishes 13 new parts in one active snapshot.
    ok(
        "INSERT INTO target (id) SELECT number FROM numbers(50000)",
        &db,
    );
    assert_eq!(ok("SELECT count(*) FROM target", &db), "count\n50003\n");
    let pointer = db.join("target.parts/.rusthouse-active");
    assert_eq!(fs::read_to_string(&pointer).unwrap(), "g1\n");

    // The old 100,000-row ceiling is gone. The whole insert commits atomically.
    ok("INSERT INTO target (id) SELECT number FROM numbers(100001)", &db);
    assert_eq!(fs::read_to_string(&pointer).unwrap(), "g2\n");
    assert_eq!(ok("SELECT count(*) FROM target", &db), "count\n150004\n");
    assert!(!db.join(".rusthouse-insert-select-0.tmp").exists());
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn inserting_from_the_same_table_reads_only_preexisting_rows() {
    let db = database();
    ok("CREATE TABLE events (id UInt64)", &db);
    ok("INSERT INTO events VALUES (1), (2)", &db);
    ok("INSERT INTO events SELECT id FROM events ORDER BY id", &db);
    assert_eq!(
        ok("SELECT id FROM events ORDER BY id", &db),
        "id\n1\n1\n2\n2\n"
    );
    assert_eq!(ok("SELECT count(*) FROM events", &db), "count\n4\n");

    // A failed source query must leave the destination unchanged.
    let result = sql("INSERT INTO events SELECT id FROM missing", &db);
    assert!(!result.status.success());
    assert_eq!(ok("SELECT count(*) FROM events", &db), "count\n4\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn type_error_in_second_part_does_not_publish_any_insert_select_rows() {
    let db = database();
    ok("CREATE TABLE target (id UInt16)", &db);
    // 61440..65535 fits UInt16; subsequent rows exceed its range.
    let result = sql(
        "INSERT INTO target SELECT number FROM numbers(61440, 5000)",
        &db,
    );
    assert!(!result.status.success(), "{result:?}");
    assert_eq!(ok("SELECT count(*) FROM target", &db), "count\n0\n");
    fs::remove_dir_all(db).unwrap();
}
