use std::process::Command;

#[test]
fn checks_unsigned_or_null_with_boundaries_and_nulls() {
    for (query, expected) in [
        ("SELECT toUInt8OrNull('255') AS value", "value\n255\n"),
        ("SELECT toUInt8OrNull('256') AS value", "value\n0\n"),
        ("SELECT toUInt16OrNull('65535') AS value", "value\n65535\n"),
        ("SELECT toUInt32OrNull('4294967295') AS value", "value\n4294967295\n"),
        ("SELECT toUInt32OrNull('-1') AS value", "value\n\\N\n"),
        ("SELECT toUInt8OrNull('invalid') AS value", "value\n\\N\n"),
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
fn rejects_invalid_unsigned_or_null_arguments() {
    for query in [
        "SELECT TOUINT8ORNULL()",
        "SELECT TOUINT8ORNULL('1', '2')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
