use std::process::Command;

#[test]
fn supports_has_token_or_null_constants() {
    for (query, expected) in [
        ("SELECT hasTokenOrNull('hello world', 'world') AS value", "value\n1\n"),
        ("SELECT hasTokenOrNull('hello world', 'wor') AS value", "value\n0\n"),
        ("SELECT hasTokenOrNull('hello world', 'bad token') AS value", "value\n\\N\n"),
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
    let query = "SELECT hasTokenOrNull('text')";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
