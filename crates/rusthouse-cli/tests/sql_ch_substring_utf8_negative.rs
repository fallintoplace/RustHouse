use std::process::Command;

#[test]
fn verifies_substring_utf8_negative_semantics() {
    for (query, expected) in [
        ("SELECT substringUTF8('aé中', -2, 1) AS value", "value\né\n"),
        ("SELECT substringUTF8('hello', -1, 1) AS value", "value\no\n"),
        ("SELECT substringUTF8('hello', 0, 2) AS value", "value\n\n"),
        ("SELECT substringUTF8('hello', -2) AS value", "value\nlo\n"),
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
    let query = "SELECT substringUTF8('abc', -1, -1)";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
