use std::process::Command;

#[test]
fn to_days_since_year_zero_supports_utc_boundaries() {
    for (query, expected) in [
        ("SELECT toDaysSinceYearZero('1970-01-01') AS value", "value\n719528\n"),
        ("SELECT toDaysSinceYearZero('2023-09-08') AS value", "value\n739136\n"),
        ("SELECT toDaysSinceYearZero('1900-01-01') AS value", "value\n693961\n"),
        ("SELECT toDaysSinceYearZero('1970-01-02 23:59:59') AS value", "value\n719529\n"),
        ("SELECT TO_DAYS('1970-01-01') AS value", "value\n719528\n"),
        ("SELECT toDaysSinceYearZero(NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn to_days_since_year_zero_rejects_invalid_inputs() {
    for query in [
        "SELECT toDaysSinceYearZero()",
        "SELECT toDaysSinceYearZero('1899-12-31')",
        "SELECT toDaysSinceYearZero('2023-02-29')",
        "SELECT toDaysSinceYearZero('2024-01-01', 'UTC')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
