use std::path::Path;
use std::process::Command;

fn run(query: &str, database: &Path) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(database)
        .output()
        .unwrap();
    assert!(output.status.success(), "{query}: {output:?}");
    output.stdout
}

#[test]
fn filters_dates_and_timestamps_at_calendar_boundaries() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-temporal-matrix-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for statement in [
        "CREATE TABLE events (id UInt64, day Date, ts DateTime)",
        "INSERT INTO events VALUES (1, '2024-02-28', '2024-02-28 23:59:59'), (2, '2024-02-29', '2024-02-29 00:00:00'), (3, '2024-02-29', '2024-02-29 23:59:59'), (4, '2024-03-01', '2024-03-01 00:00:00'), (5, '2025-03-01', '2025-03-01 01:00:00')",
    ] {
        run(statement, &database);
    }
    for (query, expected) in [
        (
            "SELECT id FROM events WHERE day BETWEEN DATE '2024-02-29' AND DATE '2024-03-01' ORDER BY id",
            "id\n2\n3\n4\n",
        ),
        (
            "SELECT id FROM events WHERE ts >= TIMESTAMP '2024-02-29 00:00:00' AND ts < TIMESTAMP '2024-03-01 00:00:00' ORDER BY id DESC",
            "id\n3\n2\n",
        ),
        (
            "SELECT count(*) FROM events WHERE day NOT BETWEEN '2024-02-29' AND '2024-03-01'",
            "count\n2\n",
        ),
        (
            "SELECT id FROM events WHERE day = '2024-02-29' ORDER BY ts DESC LIMIT 1",
            "id\n3\n",
        ),
    ] {
        assert_eq!(run(query, &database), expected.as_bytes(), "{query}");
    }
    std::fs::remove_dir_all(database).unwrap();
}
