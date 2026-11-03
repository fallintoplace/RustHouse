//! Crash injection exercises recovery after a process exit, not power-loss durability.
#[cfg(debug_assertions)]
#[test]
fn staged_insert_crash_keeps_previous_rows_and_allows_new_writes() {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let database = std::env::temp_dir().join(format!(
        "rusthouse-insert-crash-{}-{unique}",
        std::process::id()
    ));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&database)
            .output()
            .unwrap()
    };

    let created = sql("CREATE TABLE events (id UInt64)");
    assert!(created.status.success(), "{created:?}");
    let inserted = sql("INSERT INTO events VALUES (1)");
    assert!(inserted.status.success(), "{inserted:?}");

    let interrupted = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "INSERT INTO events VALUES (2)", "--database"])
        .arg(&database)
        .env("RUSTHOUSE_DEBUG_CRASH_AT", "insert-after-staging")
        .output()
        .unwrap();
    assert_eq!(interrupted.status.code(), Some(86), "{interrupted:?}");

    let after_crash = sql("SELECT id FROM events ORDER BY id");
    assert!(after_crash.status.success(), "{after_crash:?}");
    assert_eq!(after_crash.stdout, b"id\n1\n");

    let next = sql("INSERT INTO events VALUES (3)");
    assert!(next.status.success(), "{next:?}");
    let recovered = sql("SELECT id FROM events ORDER BY id");
    assert!(recovered.status.success(), "{recovered:?}");
    assert_eq!(recovered.stdout, b"id\n1\n3\n");

    fs::remove_dir_all(database).unwrap();
}
