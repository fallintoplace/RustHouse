use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

#[test]
fn update_and_delete_bind_predicates_to_the_locked_table_schema() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-locked-mutation-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&dir)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (id UInt64, note Nullable(String))",
        "INSERT INTO events VALUES (1, 'old'), (2, NULL), (3, 'keep')",
        "UPDATE events SET note = 'new' WHERE id = 1",
        "DELETE FROM events WHERE id = 2",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let rows = sql("SELECT id, note FROM events ORDER BY id");
    assert!(rows.status.success(), "{rows:?}");
    assert_eq!(rows.stdout, b"id\tnote\n1\tnew\n3\tkeep\n");
    let rejected = sql("UPDATE events SET missing = 'x' WHERE id = 1");
    assert!(!rejected.status.success());
    let rows = sql("SELECT id, note FROM events ORDER BY id");
    assert_eq!(rows.stdout, b"id\tnote\n1\tnew\n3\tkeep\n");
    fs::remove_dir_all(dir).unwrap();
}
