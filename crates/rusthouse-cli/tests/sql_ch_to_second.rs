use std::process::Command;

#[test]
fn supports_to_second_datetime_constants() {
    for (query, expected) in [
        ("SELECT toSecond('2024-10-30 23:59:07') AS value", "value\n7\n"),
        ("SELECT toSecond('2024-10-30 00:00:00') AS value", "value\n0\n"),
        ("SELECT toSecond('1969-12-31 23:59:59') AS value", "value\n59\n"),
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
    for query in ["SELECT tosecond('invalid date')"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query} unexpectedly succeeded");
    }
}
