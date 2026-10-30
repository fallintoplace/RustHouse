use std::process::Command;

fn run(sql: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse")).args(["sql", sql]).output().unwrap()
}

#[test]
fn datetrunc_supports_week_month_quarter_and_year() {
    for (sql, expected) in [
        ("SELECT dateTrunc('week', '2024-02-29 12:30:55') AS value", "2024-02-26 00:00:00"),
        ("SELECT dateTrunc('week', '2021-01-01 12:00:00') AS value", "2020-12-28 00:00:00"),
        ("SELECT dateTrunc('month', '2024-02-29 12:30:55') AS value", "2024-02-01 00:00:00"),
        ("SELECT dateTrunc('quarter', '2024-05-31 12:30:55') AS value", "2024-04-01 00:00:00"),
        ("SELECT dateTrunc('year', '2024-12-31 23:59:59') AS value", "2024-01-01 00:00:00"),
        ("SELECT dateTrunc('minutes', '2024-02-29 12:30:55') AS value", "2024-02-29 12:30:00"),
    ] {
        let result = run(sql);
        assert!(result.status.success(), "{sql}: {}", String::from_utf8_lossy(&result.stderr));
        assert_eq!(result.stdout, format!("value\n{expected}\n").as_bytes(), "{sql}");
    }
}

#[test]
fn datetrunc_rejects_bad_calendar_units() {
    for sql in [
        "SELECT dateTrunc('decade', '2024-01-01 00:00:00')",
        "SELECT dateTrunc('week', 'invalid')",
    ] {
        assert!(!run(sql).status.success(), "{sql}");
    }
}
