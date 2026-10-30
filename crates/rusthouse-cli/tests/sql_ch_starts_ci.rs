use std::process::Command;

#[test]
fn supports_starts_ci_function() {
    for (query, expected) in [
        ("SELECT startsWithCaseInsensitive('RuStHouse', 'rust') AS value", "value\n1\n"),
        ("SELECT startsWithCaseInsensitive('abc', 'abC') AS value", "value\n1\n"),
        ("SELECT startsWithCaseInsensitive('abc', 'abcd') AS value", "value\n0\n"),
        ("SELECT startsWithCaseInsensitive('Ä', 'ä') AS value", "value\n0\n"),
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
    for query in ["SELECT startsWithCaseInsensitive('a')"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query} unexpectedly succeeded");
    }
}
