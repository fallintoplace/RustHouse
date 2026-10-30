use std::process::Command;

#[test]
fn verifies_position_start_semantics() {
    for (query, expected) in [
        ("SELECT position('banana', 'na', 1) AS value", "value\n3\n"),
        ("SELECT position('banana', 'na', 4) AS value", "value\n5\n"),
        ("SELECT position('banana', 'na', 6) AS value", "value\n0\n"),
        ("SELECT position('éa', 'a', 2) AS value", "value\n3\n"),
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
    let query = "SELECT position('abc', 'a', -1)";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
