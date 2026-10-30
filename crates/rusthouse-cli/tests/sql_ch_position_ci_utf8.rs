use std::process::Command;

#[test]
fn supports_position_ci_utf8_constants() {
    for (query, expected) in [
        ("SELECT positionCaseInsensitiveUTF8('Äpfel', 'äP') AS value", "value\n1\n"),
        ("SELECT positionCaseInsensitiveUTF8('öäBC', 'Äb') AS value", "value\n2\n"),
        ("SELECT positionCaseInsensitiveUTF8('abc', 'x') AS value", "value\n0\n"),
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
    let query = "SELECT positionCaseInsensitiveUTF8('x')";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
