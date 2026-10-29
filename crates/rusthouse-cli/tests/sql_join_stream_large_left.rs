use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(db: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

fn run(db: &Path, statement: &str) -> Vec<u8> {
    let output = sql(db, statement);
    assert!(
        output.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn left_outer_join_streams_large_fact_and_preserves_null_extension() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-stream-left-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    run(&db, "CREATE TABLE facts (id UInt64)");
    run(&db, "CREATE TABLE dims (id UInt64)");
    run(&db, "INSERT INTO facts (id) SELECT number FROM numbers(33000)");
    run(&db, "INSERT INTO dims VALUES (5), (7)");

    assert_eq!(
        run(&db, "SELECT f.id, d.id FROM facts f LEFT JOIN dims d ON f.id = d.id LIMIT 3"),
        b"f.id\td.id\n0\t\\N\n1\t\\N\n2\t\\N\n"
    );
    assert_eq!(
        run(&db, "SELECT f.id, d.id FROM facts f LEFT JOIN dims d ON f.id = d.id WHERE d.id IS NOT NULL LIMIT 2"),
        b"f.id\td.id\n5\t5\n7\t7\n"
    );
    assert_eq!(
        run(&db, "SELECT f.id, d.id FROM facts f LEFT JOIN dims d ON f.id = d.id WHERE d.id IS NULL LIMIT 2"),
        b"f.id\td.id\n0\t\\N\n1\t\\N\n"
    );
    assert_eq!(
        run(&db, "SELECT count(*) FROM facts f LEFT JOIN dims d ON f.id = d.id"),
        b"count\n33000\n"
    );
    assert_eq!(
        run(&db, "SELECT count(*) FROM facts f LEFT JOIN dims d ON f.id = d.id WHERE d.id IS NULL"),
        b"count\n32998\n"
    );
    run(&db, "INSERT INTO dims VALUES (35000)");
    assert_eq!(
        run(&db, "SELECT f.id, d.id FROM facts f FULL JOIN dims d ON f.id = d.id WHERE f.id IS NULL LIMIT 2"),
        b"f.id\td.id\n\\N\t35000\n"
    );
    assert_eq!(
        run(&db, "SELECT * FROM facts f FULL JOIN dims d USING (id) WHERE f.id IS NULL LIMIT 2"),
        b"f.id\n35000\n"
    );
    assert_eq!(
        run(&db, "SELECT count(*) FROM facts f FULL JOIN dims d ON f.id = d.id"),
        b"count\n33001\n"
    );
    assert_eq!(
        run(&db, "SELECT count(*) FROM facts f FULL JOIN dims d ON f.id = d.id WHERE f.id IS NULL"),
        b"count\n1\n"
    );
    fs::remove_dir_all(db).unwrap();
}
