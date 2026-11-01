use std::process::Command;

fn run(sql: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql])
        .output()
        .unwrap()
}

#[test]
fn last_day_of_week_supports_clickhouse_modes() {
    for (query, expected) in [
        ("SELECT toLastDayOfWeek('2024-01-03') AS day", "2024-01-06"),
        (
            "SELECT toLastDayOfWeek('2024-01-03', 0) AS day",
            "2024-01-06",
        ),
        (
            "SELECT toLastDayOfWeek('2024-01-03', 1) AS day",
            "2024-01-07",
        ),
        (
            "SELECT toLastDayOfWeek('2024-01-03', 8) AS day",
            "2024-01-06",
        ),
        (
            "SELECT toLastDayOfWeek('2024-01-03', 9) AS day",
            "2024-01-07",
        ),
        (
            "SELECT toLastDayOfWeek('2024-01-03 20:30:00', 3) AS day",
            "2024-01-07",
        ),
    ] {
        let result = run(query);
        assert!(
            result.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            result.stdout,
            format!("day\n{expected}\n").as_bytes(),
            "{query}"
        );
    }
}

#[test]
fn last_day_of_week_rejects_out_of_range_modes() {
    for sql in [
        "SELECT toLastDayOfWeek('2024-01-03', 10)",
        "SELECT toLastDayOfWeek('2024-01-03', -1)",
    ] {
        assert!(!run(sql).status.success(), "{sql}");
    }
}
