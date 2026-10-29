use std::process::Command;

#[test]
fn checks_signed_or_null_with_boundaries_and_nulls() {
    for (query, expected) in [
        ("SELECT toInt8OrNull('128') AS value", "value\n-128\n"),
        ("SELECT toInt16OrNull('65535') AS value", "value\n-1\n"),
        ("SELECT toInt32OrNull('4294967295') AS value", "value\n-1\n"),
        ("SELECT toInt8OrNull('not_a_number') AS value", "value\n\\N\n"),
        ("SELECT toInt32OrNull(NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .expect("execute RustHouse SQL");
        assert!(
            output.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn rejects_invalid_signed_or_null_arguments() {
    for query in [
        "SELECT TOINT8ORNULL()",
        "SELECT TOINT8ORNULL('1', '2')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
