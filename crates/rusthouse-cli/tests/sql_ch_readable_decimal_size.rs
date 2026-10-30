use std::process::Command;

#[test]
fn readable_decimal_size_reports_correct_values() {
    for (query, expected) in [
        ("SELECT formatReadableDecimalSize(1) AS value", "value\n1.00 B\n"),
        ("SELECT formatReadableDecimalSize(1000) AS value", "value\n1.00 KB\n"),
        ("SELECT formatReadableDecimalSize(1024) AS value", "value\n1.02 KB\n"),
        ("SELECT formatReadableDecimalSize(1000000) AS value", "value\n1.00 MB\n"),
        ("SELECT formatReadableDecimalSize(1536,1) AS value", "value\n1.5 KB\n"),
        ("SELECT formatReadableDecimalSize(NULL) AS value", "value\n\\N\n"),
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
fn readable_decimal_size_rejects_bad_inputs() {
    for query in [
        "SELECT formatReadableDecimalSize()",
        "SELECT formatReadableDecimalSize(123,-1)",
        "SELECT formatReadableDecimalSize(123,9)",
        "SELECT formatReadableDecimalSize(1,2,3)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
