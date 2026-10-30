use std::process::Command;

#[test]
fn verifies_substring_byte_utf8_boundaries() {
    for (query, expected) in [
        ("SELECT substring('éa', 1, 2) AS value", "value\né\n"),
        ("SELECT substringUTF8('éa', 1, 2) AS value", "value\néa\n"),
        ("SELECT substring('éa', 3, 1) AS value", "value\na\n"),
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
    let query = "SELECT substring('éa', 2, 1)";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
