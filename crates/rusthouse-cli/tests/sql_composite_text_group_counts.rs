use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(query: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn repeated_nullable_text_composite_keys_aggregate_without_collisions() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-composite-text-count-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for statement in [
        "CREATE TABLE events (region Nullable(String), segment String, flag UInt8)",
        "INSERT INTO events VALUES ('a', 'bc', 1), ('ab', 'c', 1), (NULL, 'bc', 1), ('', 'bc', 1)",
        "INSERT INTO events VALUES ('a', 'bc', 1), ('ab', 'c', 0), (NULL, 'bc', 0), ('', 'bc', 1)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT region, segment, count(*) FROM events GROUP BY region, segment",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"region\tsegment\tcount\na\tbc\t2\nab\tc\t2\nNULL\tbc\t2\n\tbc\t2\n"
    );
    let filtered = sql(
        "SELECT region, segment, count(*) FROM events WHERE flag = 1 GROUP BY region, segment",
        &db,
    );
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(
        filtered.stdout,
        b"region\tsegment\tcount\na\tbc\t2\nab\tc\t1\nNULL\tbc\t1\n\tbc\t2\n"
    );
    fs::remove_dir_all(db).unwrap();
}
