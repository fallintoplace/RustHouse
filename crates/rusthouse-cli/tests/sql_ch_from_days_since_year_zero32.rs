use std::process::Command;

#[test]
fn from_days_since_year_zero32_supports_utc_boundaries() {
    for (query, expected) in [
        ("SELECT fromDaysSinceYearZero32(693961) AS value", "value\n1900-01-01\n"),
        ("SELECT fromDaysSinceYearZero32(719528) AS value", "value\n1970-01-01\n"),
        ("SELECT fromDaysSinceYearZero32(739136) AS value", "value\n2023-09-08\n"),
        ("SELECT fromDaysSinceYearZero32(840057) AS value", "value\n2299-12-31\n"),
        ("SELECT fromDaysSinceYearZero32(NULL) AS value", "value\n\\N\n"),
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
fn from_days_since_year_zero32_rejects_invalid_inputs() {
    for query in [
        "SELECT fromDaysSinceYearZero32()",
        "SELECT fromDaysSinceYearZero32(693960)",
        "SELECT fromDaysSinceYearZero32(840058)",
        "SELECT fromDaysSinceYearZero32(719528, 1)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
