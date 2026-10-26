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
    let too_many = sql(
        "INSERT INTO target (id) SELECT number FROM numbers(4097)",
        &db,
    );
    assert!(!too_many.status.success());
    assert_eq!(ok("SELECT count(*) FROM target", &db), "count\n3\n");
    fs::remove_dir_all(db).unwrap();
}
