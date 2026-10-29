use std::process::Command;

#[test]
fn readable_size_reports_correct_values() {
    for (query, expected) in [
        ("SELECT formatReadableSize(1) AS value", "value\n1.00 B\n"),
        ("SELECT formatReadableSize(1024) AS value", "value\n1.00 KiB\n"),
        ("SELECT formatReadableSize(1536) AS value", "value\n1.50 KiB\n"),
        ("SELECT formatReadableSize(1048576) AS value", "value\n1.00 MiB\n"),
        ("SELECT formatReadableSize(192851925,0) AS value", "value\n184 MiB\n"),
        ("SELECT formatReadableSize(-2048) AS value", "value\n-2.00 KiB\n"),
        ("SELECT formatReadableSize(NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .expect("run SQL");
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn readable_size_rejects_bad_inputs() {
    for query in [
        "SELECT formatReadableSize()",
        "SELECT formatReadableSize(123, -1)",
        "SELECT formatReadableSize(123, 9)",
        "SELECT formatReadableSize(123, 2, 4)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
