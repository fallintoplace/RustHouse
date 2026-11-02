use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(query: &str, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn chained_join_preserves_temporal_projected_types() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-chained-join-date-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&database).unwrap();
    for query in [
        "CREATE TABLE events (id UInt64, owner UInt64, visited Date, historic Date32, visited_time DateTime)",
        "CREATE TABLE owners (id UInt64, region UInt64)",
        "CREATE TABLE regions (id UInt64, label String)",
        "INSERT INTO events VALUES (1, 4, '2024-01-03', '1969-12-31', '2024-01-03 12:15:01'), (2, 4, '2024-01-01', '1970-01-01', '2024-01-01 09:00:00')",
        "INSERT INTO owners VALUES (4, 8)",
        "INSERT INTO regions VALUES (8, 'north')",
    ] {
        let result = sql(query, &database);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    let result = sql(
        "SELECT e.visited, e.historic, e.visited_time, r.label FROM events e JOIN owners o ON e.owner = o.id JOIN regions r ON o.region = r.id ORDER BY e.visited",
        &database,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout,
        b"e.visited\te.historic\te.visited_time\tr.label\n2024-01-01\t1970-01-01\t2024-01-01 09:00:00\tnorth\n2024-01-03\t1969-12-31\t2024-01-03 12:15:01\tnorth\n");
    fs::remove_dir_all(database).unwrap();
}
