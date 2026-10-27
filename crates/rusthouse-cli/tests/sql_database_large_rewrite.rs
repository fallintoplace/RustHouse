use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn update_and_delete_stream_more_than_25000_rows() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-large-rewrite-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let create = sql("CREATE TABLE events (id UInt64)", &db);
    assert!(create.status.success(), "{create:?}");
    for _ in 0..9 {
        let insert = sql("INSERT INTO events SELECT number FROM numbers(3500)", &db);
        assert!(insert.status.success(), "{insert:?}");
    }
    assert_eq!(
        sql("SELECT count(*) FROM events", &db).stdout,
        b"count\n31500\n"
    );

    let update = sql("UPDATE events SET id = 4 WHERE id = 3", &db);
    assert!(update.status.success(), "{update:?}");
    assert!(String::from_utf8_lossy(&update.stdout).contains("Updated 9 rows"));
    assert_eq!(
        sql("SELECT count(*) FROM events WHERE id = 3", &db).stdout,
        b"count\n0\n"
    );
    assert_eq!(
        sql("SELECT count(*) FROM events WHERE id = 4", &db).stdout,
        b"count\n18\n"
    );

    let removed = sql("DELETE FROM events WHERE id >= 3000", &db);
    assert!(removed.status.success(), "{removed:?}");
    assert!(String::from_utf8_lossy(&removed.stdout).contains("Deleted 4500 rows"));
    assert_eq!(
        sql("SELECT count(*) FROM events", &db).stdout,
        b"count\n27000\n"
    );

    let append = sql("INSERT INTO events VALUES (5000)", &db);
    assert!(append.status.success(), "{append:?}");
    assert_eq!(
        sql("SELECT count(*) FROM events", &db).stdout,
        b"count\n27001\n"
    );
    fs::remove_dir_all(db).unwrap();
}
