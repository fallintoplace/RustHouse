use std::process::Command;

#[test]
fn supports_substr_and_mid_aliases() {
    for (query, expected) in [
        ("SELECT substr('hello', 2, 3) AS value", "value\nell\n"),
        ("SELECT mid('éa', 1, 2) AS value", "value\né\n"),
        ("SELECT substr('hello', 2) AS value", "value\nello\n"),
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
    let query = "SELECT mid('éa', 2, 1)";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
