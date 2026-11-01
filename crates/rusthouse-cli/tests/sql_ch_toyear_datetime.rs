use std::process::Command;

#[test]
fn toyear_accepts_date_datetime_and_utc_boundary_offsets() {
    for (query, year) in [
        ("SELECT toYear('1970-01-01') AS year", 1970),
        ("SELECT toYear('2149-06-06') AS year", 2149),
        ("SELECT toYear('2024-02-29 20:05:06') AS year", 2024),
        ("SELECT toYear('2024-12-31 23:30:00-02:00') AS year", 2025),
        ("SELECT toYear('2025-01-01 00:30:00+02:00') AS year", 2024),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            result.stdout,
            format!("year\n{year}\n").as_bytes(),
            "{query}"
        );
    }
}

#[test]
fn toyear_rejects_invalid_dates() {
    for sql in ["SELECT toYear('2024-02-30')", "SELECT toYear('invalid')"] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", sql])
            .output()
            .unwrap();
        assert!(!result.status.success(), "{sql}");
    }
}
