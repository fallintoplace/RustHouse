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
fn grouped_join_renders_date_group_keys_as_dates() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-grouped-join-date-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    for statement in [
        "CREATE TABLE events (id UInt64, day Date)",
        "CREATE TABLE owners (id UInt64)",
        "INSERT INTO events VALUES (1, '2024-01-03'), (2, '2024-01-03'), (3, '2024-01-01')",
        "INSERT INTO owners VALUES (1), (2), (3)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT e.day, count(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY e.day ORDER BY e.day",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"e.day\tcount\n2024-01-01\t1\n2024-01-03\t2\n");
    fs::remove_dir_all(db).unwrap();
}
