use std::process::Command;

#[test]
fn verifies_is_not_null_behavior() {
    for (query, expected) in [
        ("SELECT isNotNull(NULL) AS value", "value\n0\n"),
        ("SELECT isNotNull('') AS value", "value\n1\n"),
        ("SELECT isNotNull(-1) AS value", "value\n1\n"),
        ("SELECT isNotNull(nullIf(1, 1)) AS value", "value\n0\n"),
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
    let query = "SELECT isNotNull()";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
