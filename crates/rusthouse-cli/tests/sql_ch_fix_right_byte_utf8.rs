use std::process::Command;

#[test]
fn verifies_right_byte_utf8_boundaries() {
    for (query, expected) in [
        ("SELECT right('aé', 2) AS value", "value\né\n"),
        ("SELECT rightUTF8('éa', 2) AS value", "value\néa\n"),
        ("SELECT right('éa', 1) AS value", "value\na\n"),
        ("SELECT right('x🙂', 4) AS value", "value\n🙂\n"),
        ("SELECT rightUTF8('x🙂', 2) AS value", "value\nx🙂\n"),
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
    let query = "SELECT right('éa', 2)";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
