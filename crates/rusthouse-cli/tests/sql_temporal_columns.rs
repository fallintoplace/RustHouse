use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn query(dir: &Path, statement: &str) -> Vec<u8> {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(dir)
        .output()
        .unwrap();
    assert!(result.status.success(), "query failed: {result:?}");
    result.stdout
}

#[test]
fn dates_and_timestamps_round_trip_and_filter_correctly() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-temporal-roundtrip-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    query(&dir, "CREATE TABLE events (d Date, ts DateTime)");
    query(
        &dir,
        "INSERT INTO events VALUES ('2013-07-01', '2013-07-01 12:34:56'), ('2013-07-14', '2013-07-14 23:59:59')",
    );
    assert_eq!(
        query(&dir, "SELECT d, ts FROM events ORDER BY d"),
        b"d\tts\n2013-07-01\t2013-07-01 12:34:56\n2013-07-14\t2013-07-14 23:59:59\n"
    );
    assert_eq!(
        query(&dir, "SELECT count(*) FROM events WHERE d >= '2013-07-14'"),
        b"count\n1\n"
    );
    assert_eq!(
        query(&dir, "SELECT max(d) FROM events"),
        b"max(d)\n2013-07-14\n"
    );
    assert_eq!(
        query(&dir, "SELECT min(ts) FROM events"),
        b"min(ts)\n2013-07-01 12:34:56\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
