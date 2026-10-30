use std::process::Command;

#[test]
fn supports_is_valid_ascii_constants() {
    for (query, expected) in [
        ("SELECT isValidASCII('Plain ASCII') AS value", "value\n1\n"),
        ("SELECT isValidASCII('café') AS value", "value\n0\n"),
        ("SELECT isValidASCII('') AS value", "value\n1\n"),
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
    let query = "SELECT isValidASCII()";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
