use std::process::Command;

fn output(query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output().unwrap()
}

#[test]
fn start_of_week_supports_sunday_and_monday_modes() {
    for (query, day) in [
        ("SELECT toStartOfWeek('2024-01-03') AS day", "2023-12-31"),
        ("SELECT toStartOfWeek('2024-01-03', 0) AS day", "2023-12-31"),
        ("SELECT toStartOfWeek('2024-01-03', 1) AS day", "2024-01-01"),
        ("SELECT toStartOfWeek('2024-01-03', 8) AS day", "2023-12-31"),
        ("SELECT toStartOfWeek('2024-01-03', 9) AS day", "2024-01-01"),
        ("SELECT toStartOfWeek('1970-01-01', 0) AS day", "1969-12-28"),
        ("SELECT toStartOfWeek('2024-01-03 23:59:59', 1) AS day", "2024-01-01"),
    ] {
        let result = output(query);
        assert!(result.status.success(), "{query}: {}", String::from_utf8_lossy(&result.stderr));
        assert_eq!(result.stdout, format!("day\n{day}\n").as_bytes(), "{query}");
    }
}

#[test]
fn start_of_week_handles_null_and_rejects_bad_modes() {
    assert_eq!(output("SELECT toStartOfWeek(NULL) AS day").stdout, b"day\n\\N\n");
    assert_eq!(output("SELECT toStartOfWeek('2024-01-03', NULL) AS day").stdout, b"day\n\\N\n");
    for query in [
        "SELECT toStartOfWeek('2024-01-03', -1)",
        "SELECT toStartOfWeek('2024-01-03', 10)",
        "SELECT toStartOfWeek('2024-01-03', 1, 2)",
    ] {
        assert!(!output(query).status.success(), "{query}");
    }
}
