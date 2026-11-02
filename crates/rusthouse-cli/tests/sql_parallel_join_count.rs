use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, db: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(output.status.success(), "{statement}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn parallel_inner_count_probe_preserves_duplicate_rows() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-join-count-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (id UInt64)", &db);
    sql("CREATE TABLE dims (id UInt64)", &db);
    sql("INSERT INTO dims VALUES (1), (1), (2)", &db);
    for _ in 0..40 {
        sql("INSERT INTO facts VALUES (1), (2), (3)", &db);
    }
    assert_eq!(
        sql("SELECT count(*) FROM facts f INNER JOIN dims d ON f.id = d.id", &db),
        "count\n120\n"
    );
    assert_eq!(
        sql(
            "SELECT count(*) FROM facts f INNER JOIN dims d ON f.id = d.id WHERE f.id IS NULL",
            &db
        ),
        "count\n0\n"
    );
    fs::remove_dir_all(db).unwrap();
}
