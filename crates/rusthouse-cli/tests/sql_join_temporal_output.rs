use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db).output().unwrap()
}

#[test]
fn two_table_joins_render_dates_in_streamed_and_ordered_results() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-temporal-{}-{}", std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    for query in [
        "CREATE TABLE events (id UInt64, d Date, ts DateTime)",
        "CREATE TABLE owners (id UInt64, d32 Date32)",
        "INSERT INTO events VALUES (1, '2024-01-03', '2024-01-03 14:20:00'), (2, '2024-01-01', '2024-01-01 09:10:00')",
        "INSERT INTO owners VALUES (1, '1969-12-31'), (2, '1970-01-01')",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    for query in [
        "SELECT e.d, e.ts, o.d32 FROM events e JOIN owners o ON e.id = o.id ORDER BY e.id",
        "SELECT e.d, e.ts, o.d32 FROM events e JOIN owners o ON e.id = o.id",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        let rows = String::from_utf8(result.stdout).unwrap();
        assert!(rows.contains("2024-01-03\t2024-01-03 14:20:00\t1969-12-31"), "{query}: {rows}");
        assert!(rows.contains("2024-01-01\t2024-01-01 09:10:00\t1970-01-01"), "{query}: {rows}");
    }
    fs::remove_dir_all(db).unwrap();
}
