use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn streams_large_right_table_with_small_left_hash_build() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-large-right-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for statement in [
        "CREATE TABLE fact (id UInt64)",
        "CREATE TABLE events (id UInt64)",
        "INSERT INTO fact VALUES (120000)",
        "INSERT INTO events SELECT number FROM numbers(40000)",
        "INSERT INTO events SELECT number FROM numbers(40000, 40000)",
        "INSERT INTO events SELECT number FROM numbers(80000, 40001)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    let query = "SELECT f.id, e.id FROM fact f INNER JOIN events e ON f.id = e.id LIMIT 1";
    let joined = sql(query, &db);
    assert!(joined.status.success(), "{joined:?}");
    assert_eq!(joined.stdout, b"f.id\te.id\n120000\t120000\n");
    fs::remove_dir_all(db).unwrap();
}
