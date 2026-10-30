use std::process::Command;

#[test]
fn supports_to_minute_datetime_constants() {
    for (query, expected) in [
        ("SELECT toMinute('2024-10-30 23:59:07') AS value", "value\n59\n"),
        ("SELECT toMinute('2024-10-30 00:01:07') AS value", "value\n1\n"),
        ("SELECT toMinute('1969-12-31 23:59:59') AS value", "value\n59\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    for query in ["SELECT tominute('invalid date')"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query} unexpectedly succeeded");
    }
}
