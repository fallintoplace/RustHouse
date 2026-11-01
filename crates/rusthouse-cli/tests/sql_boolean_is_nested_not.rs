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

fn ok(db: &Path, statement: &str) -> String {
    let output = sql(db, statement);
    assert!(
        output.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn negating_is_boolean_keeps_null_rows_in_sql_where() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-is-boolean-negation-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();

    ok(
        &db,
        "CREATE TABLE events (id UInt64, value Nullable(UInt64))",
    );
    ok(
        &db,
        "INSERT INTO events VALUES (1, NULL), (2, 0), (3, 1), (4, 2)",
    );

    for (predicate, expected) in [
        ("value IS TRUE", 2),
        ("value IS FALSE", 1),
        ("value IS NOT TRUE", 2),
        ("value IS NOT FALSE", 3),
        ("NOT (value IS TRUE)", 2),
        ("NOT (value IS FALSE)", 3),
        ("NOT (value IS NOT TRUE)", 2),
        ("NOT (value IS NOT FALSE)", 1),
        ("NOT (value IS TRUE OR value IS FALSE)", 1),
        ("NOT (value IS TRUE AND id >= 3)", 2),
    ] {
        let statement = format!("SELECT count(*) FROM events WHERE {predicate}");
        assert_eq!(
            ok(&db, &statement),
            format!("count\n{expected}\n"),
            "{predicate}"
        );
    }

    fs::remove_dir_all(db).unwrap();
}
