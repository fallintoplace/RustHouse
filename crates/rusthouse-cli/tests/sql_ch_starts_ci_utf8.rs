use std::process::Command;

#[test]
fn supports_starts_ci_utf8_function() {
    for (query, expected) in [
        ("SELECT startsWithCaseInsensitiveUTF8('Äpfel', 'äP') AS value", "value\n1\n"),
        ("SELECT startsWithCaseInsensitiveUTF8('你好世界', '你好') AS value", "value\n1\n"),
        ("SELECT startsWithCaseInsensitiveUTF8('Apple', 'banana') AS value", "value\n0\n"),
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
    for query in [
        "SELECT startsWithCaseInsensitiveUTF8('a')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query} unexpectedly succeeded");
    }
}
