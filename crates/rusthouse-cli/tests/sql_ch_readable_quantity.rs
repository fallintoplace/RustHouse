use std::process::Command;

#[test]
fn readable_quantity_reports_correct_values() {
    for (query, expected) in [
        ("SELECT formatReadableQuantity(0) AS value", "value\n0.00\n"),
        ("SELECT formatReadableQuantity(1000) AS value", "value\n1.00 thousand\n"),
        ("SELECT formatReadableQuantity(12345) AS value", "value\n12.35 thousand\n"),
        ("SELECT formatReadableQuantity(1000000) AS value", "value\n1.00 million\n"),
        ("SELECT formatReadableQuantity(1500,1) AS value", "value\n1.5 thousand\n"),
        ("SELECT formatReadableQuantity(NULL) AS value", "value\n\\N\n"),
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
fn readable_quantity_rejects_bad_inputs() {
    for query in [
        "SELECT formatReadableQuantity()",
        "SELECT formatReadableQuantity(10,-1)",
        "SELECT formatReadableQuantity(10,9)",
        "SELECT formatReadableQuantity(1,2,3)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
