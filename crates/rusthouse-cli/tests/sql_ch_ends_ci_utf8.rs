use std::process::Command;

#[test]
fn supports_ends_ci_utf8_function() {
    for (query, expected) in [
        ("SELECT endsWithCaseInsensitiveUTF8('GROẞ', 'groß') AS value", "value\n1\n"),
        ("SELECT endsWithCaseInsensitiveUTF8('Grüße', 'ÜẞE') AS value", "value\n1\n"),
        ("SELECT endsWithCaseInsensitiveUTF8('MÜNCHEN', 'chen') AS value", "value\n1\n"),
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
    for query in ["SELECT endsWithCaseInsensitiveUTF8('a')"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query} unexpectedly succeeded");
    }
}
