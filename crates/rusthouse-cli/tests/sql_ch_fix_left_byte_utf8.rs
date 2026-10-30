use std::process::Command;

#[test]
fn verifies_left_byte_utf8_boundaries() {
    for (query, expected) in [
        ("SELECT left('éa', 2) AS value", "value\né\n"),
        ("SELECT leftUTF8('éa', 2) AS value", "value\néa\n"),
        ("SELECT left('aé', 1) AS value", "value\na\n"),
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
    let query = "SELECT left('éa', 1)";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
