use std::process::Command;

#[test]
fn verifies_is_null_behavior() {
    for (query, expected) in [
        ("SELECT isNull(NULL) AS value", "value\n1\n"),
        ("SELECT isNull('') AS value", "value\n0\n"),
        ("SELECT isNull(0) AS value", "value\n0\n"),
        ("SELECT isNull(nullIf(1, 1)) AS value", "value\n1\n"),
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
    let query = "SELECT isNull()";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
