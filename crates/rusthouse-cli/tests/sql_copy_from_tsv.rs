use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-copy-tsv-{}-{}",
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
fn imports_tsv_with_clickhouse_nulls_and_escapes() {
    let db = database();
    ok(
        "CREATE TABLE events (id UInt64, name String, note Nullable(String))",
        &db,
    );
    let input = db.join("input.tsv");
    fs::write(
        &input,
        "name\tid\tnote\nAda\\tLovelace\t1\t\\N\nGrace\\nHopper\t2\t\n",
    )
    .unwrap();
    let statement = format!(
        "COPY events FROM '{}' WITH (FORMAT TSV, HEADER TRUE)",
        input.display()
    );
    ok(&statement, &db);
    assert_eq!(
        ok("SELECT id, name, note FROM events ORDER BY id", &db),
        "id\tname\tnote\n1\tAda\\tLovelace\t\\N\n2\tGrace\\nHopper\t\n"
    );
    fs::write(&input, "name\tid\tnote\ninvalid\\x\t3\t\\N\n").unwrap();
    assert!(!sql(&statement, &db).status.success());
    assert_eq!(ok("SELECT count(*) FROM events", &db), "count\n2\n");
    fs::remove_dir_all(db).unwrap();
}
