//! Keep numeric and typed SELECT semantics consistent across output paths.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-cross-type-contract-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    dir
}

fn sql(dir: &Path, statement: &str, format: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rusthouse"));
    cmd.args(["sql", statement, "--database"]).arg(dir);
    if let Some(format) = format {
        cmd.args(["--format", format]);
    }
    cmd.output().unwrap()
}

fn ok(dir: &Path, statement: &str, format: Option<&str>) -> String {
    let result = sql(dir, statement, format);
    assert!(
        result.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn nullable_typed_columns_keep_rows_and_nulls_in_tsv_and_jsonl() {
    let db = database();
    ok(
        &db,
        "CREATE TABLE entries (id UInt64, name Nullable(String), score Nullable(UInt64))",
        None,
    );
    ok(&db, "INSERT INTO entries VALUES (1, 'Ada', 10), (2, NULL, NULL), (3, 'Bob', 0)", None);

    assert_eq!(
        ok(&db, "SELECT id, name FROM entries WHERE id >= 2 ORDER BY id", None),
        "id\tname\n2\t\\N\n3\tBob\n"
    );
    let jsonl = ok(
        &db,
        "SELECT id, name FROM entries WHERE id >= 2 ORDER BY id",
        Some("jsonl"),
    );
    assert!(jsonl.contains("\"id\":\"2\""), "{jsonl}");
    assert!(jsonl.contains("\"name\":null"), "{jsonl}");
    assert!(jsonl.contains("\"id\":\"3\""), "{jsonl}");
    assert!(jsonl.contains("\"name\":\"Bob\""), "{jsonl}");

    assert_eq!(
        ok(&db, "SELECT id FROM entries WHERE score IS NULL ORDER BY id", None),
        "id\n2\n"
    );
    assert_eq!(
        ok(&db, "SELECT count(name) FROM entries", None),
        "count(name)\n2\n"
    );
    let failed = sql(&db, "INSERT INTO entries VALUES (NULL, 'Bad', 12)", None);
    assert!(!failed.status.success());
    assert_eq!(ok(&db, "SELECT count(*) FROM entries", None), "count\n3\n");
    fs::remove_dir_all(db).unwrap();
}
