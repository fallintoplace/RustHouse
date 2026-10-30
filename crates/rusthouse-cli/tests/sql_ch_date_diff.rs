use std::process::Command;

#[test]
fn date_diff_supports_boundaries() {
    for (query, expected) in [
        ("SELECT dateDiff('day','2024-01-01 23:59:59','2024-01-02 00:00:00') AS value", "value\n1\n"),
        ("SELECT dateDiff('week','2024-01-07','2024-01-08') AS value", "value\n1\n"),
        ("SELECT dateDiff('month','2023-01-31','2023-02-01') AS value", "value\n1\n"),
        ("SELECT dateDiff('year','2023-12-31','2024-01-01') AS value", "value\n1\n"),
        ("SELECT dateDiff('quarter','2023-03-31','2023-04-01') AS value", "value\n1\n"),
        ("SELECT dateDiff('minute','2024-01-01 00:00:59','2024-01-01 00:01:00') AS value", "value\n1\n"),
        ("SELECT dateDiff('day','2024-01-02','2024-01-01') AS value", "value\n-1\n"),
        ("SELECT dateDiff('second','2024-01-01','2024-01-02') AS value", "value\n86400\n"),
        ("SELECT DATE_DIFF('day','2024-01-01','2024-01-02') AS value", "value\n1\n"),
        ("SELECT dateDiff('day',NULL,'2024-01-01') AS value", "value\n\\N\n"),
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
fn date_diff_rejects_unsupported_inputs() {
    for query in [
        "SELECT dateDiff()",
        "SELECT dateDiff('week','2024-01-01')",
        "SELECT dateDiff('century','2024-01-01','2024-01-02')",
        "SELECT dateDiff('week','2023-02-29','2024-01-01')",
        "SELECT dateDiff('week','2024-01-01','2024-01-02','UTC')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
