use std::process::Command;

#[test]
fn unix_milli_converts_utc_seconds_exactly() {
    for (query, expected) in [
        ("SELECT TOUNIXTIMESTAMP64MILLI('1970-01-01 00:00:00') AS value", "value\n0\n"),
        ("SELECT TOUNIXTIMESTAMP64MILLI('1970-01-01 00:00:01') AS value", "value\n1000\n"),
        ("SELECT TOUNIXTIMESTAMP64MILLI('1969-12-31 23:59:59') AS value", "value\n-1000\n"),
        ("SELECT TOUNIXTIMESTAMP64MILLI('2024-01-01 00:00:00') AS value", "value\n1704067200000\n"),
        ("SELECT TOUNIXTIMESTAMP64MILLI(NULL) AS value", "value\n\\N\n"),
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
fn unix_milli_rejects_invalid_dates_and_overflow() {
    for query in [
        "SELECT TOUNIXTIMESTAMP64MILLI('2023-02-29')",
        "SELECT TOUNIXTIMESTAMP64MILLI('2300-01-01')",
        "SELECT TOUNIXTIMESTAMP64MILLI()",
        "SELECT TOUNIXTIMESTAMP64MILLI('2024-01-01', 'UTC')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
