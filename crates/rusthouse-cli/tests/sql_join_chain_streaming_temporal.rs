use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(query: &str, dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn streaming_join_chains_use_source_types_for_output() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-streaming-date-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    for statement in [
        "CREATE TABLE events (id UInt64, owner UInt64, event_date Date, event_time DateTime)",
        "CREATE TABLE owners (id UInt64, region UInt64, birthday Date32)",
        "CREATE TABLE regions (id UInt64, label String)",
        "INSERT INTO events VALUES (1, 9, '2024-01-03', '2024-01-03 12:00:01')",
        "INSERT INTO owners VALUES (9, 2, '1969-12-31')",
        "INSERT INTO regions VALUES (2, 'east')",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    // An unordered INNER chain uses the streaming executor.
    let result = sql(
        "SELECT e.event_date, e.event_time, o.birthday, r.label FROM events e JOIN owners o ON e.owner = o.id JOIN regions r ON o.region = r.id LIMIT 5",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"e.event_date\te.event_time\to.birthday\tr.label\n2024-01-03\t2024-01-03 12:00:01\t1969-12-31\teast\n"
    );
    fs::remove_dir_all(db).unwrap();
}
