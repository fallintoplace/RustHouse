use std::process::Command;

#[test]
fn checks_signed_or_zero_with_boundaries_and_nulls() {
    for (query, expected) in [
        ("SELECT toInt8OrZero('128') AS value", "value\n-128\n"),
        ("SELECT toInt16OrZero('65535') AS value", "value\n-1\n"),
        ("SELECT toInt32OrZero('4294967295') AS value", "value\n-1\n"),
        ("SELECT toInt64OrZero('42') AS value", "value\n42\n"),
        ("SELECT toInt64OrZero('broken') AS value", "value\n0\n"),
        ("SELECT toInt8OrZero(NULL) AS value", "value\n\\N\n"),
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
fn rejects_invalid_signed_or_zero_arguments() {
    for query in [
        "SELECT TOINT8ORZERO()",
        "SELECT TOINT8ORZERO('1', '2')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
