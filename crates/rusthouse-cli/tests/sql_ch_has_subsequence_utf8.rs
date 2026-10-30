use std::process::Command;

#[test]
fn supports_has_subsequence_utf8_constants() {
    for (query, expected) in [
        ("SELECT hasSubsequenceUTF8('aé水z', 'éz') AS value", "value\n1\n"),
        ("SELECT hasSubsequenceUTF8('aé', 'éa') AS value", "value\n0\n"),
        ("SELECT hasSubsequenceUTF8('aé水z', '') AS value", "value\n1\n"),
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
    let query = "SELECT hasSubsequenceUTF8('x')";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
