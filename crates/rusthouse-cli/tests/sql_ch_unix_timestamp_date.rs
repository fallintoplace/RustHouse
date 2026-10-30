use std::process::Command;

#[test]
fn to_unix_timestamp_supports_date_and_datetime_constants() {
    for (sql, seconds) in [
        ("SELECT toUnixTimestamp('1970-01-01') AS value", 0_i64),
        ("SELECT toUnixTimestamp('1970-01-02') AS value", 86_400),
        ("SELECT toUnixTimestamp('2024-02-29') AS value", 1_709_164_800),
        ("SELECT toUnixTimestamp('2024-02-29 00:00:00') AS value", 1_709_164_800),
        ("SELECT toUnixTimestamp('2149-06-06') AS value", 5_662_224_000),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", sql])
            .output()
            .unwrap();
        assert!(result.status.success(), "{sql}: {}", String::from_utf8_lossy(&result.stderr));
        assert_eq!(result.stdout, format!("value\n{seconds}\n").as_bytes(), "{sql}");
    }
}

#[test]
fn to_unix_timestamp_rejects_invalid_calendar_dates() {
    for sql in [
        "SELECT toUnixTimestamp('2023-02-29')",
        "SELECT toUnixTimestamp('not a date')",
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", sql])
            .output()
            .unwrap();
        assert!(!result.status.success(), "{sql}");
    }
}
