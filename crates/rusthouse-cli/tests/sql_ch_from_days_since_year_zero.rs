use std::process::Command;

#[test]
fn from_days_since_year_zero_supports_utc_boundaries() {
    for (query, expected) in [
        ("SELECT fromDaysSinceYearZero(719528) AS value", "value\n1970-01-01\n"),
        ("SELECT fromDaysSinceYearZero(719529) AS value", "value\n1970-01-02\n"),
        ("SELECT fromDaysSinceYearZero(739136) AS value", "value\n2023-09-08\n"),
        ("SELECT fromDaysSinceYearZero(785063) AS value", "value\n2149-06-06\n"),
        ("SELECT FROM_DAYS(719528) AS value", "value\n1970-01-01\n"),
        ("SELECT fromDaysSinceYearZero(NULL) AS value", "value\n\\N\n"),
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
fn from_days_since_year_zero_rejects_invalid_inputs() {
    for query in [
        "SELECT fromDaysSinceYearZero()",
        "SELECT fromDaysSinceYearZero(719527)",
        "SELECT fromDaysSinceYearZero(785064)",
        "SELECT fromDaysSinceYearZero(-1)",
        "SELECT fromDaysSinceYearZero(719528,719529)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
