use std::process::Command;

#[test]
fn verifies_binary_representation_semantics() {
    for (query, expected) in [
        ("SELECT bin('A') AS value", "value\n01000001\n"),
        ("SELECT bin('é') AS value", "value\n1100001110101001\n"),
        ("SELECT bin(257) AS value", "value\n0000000100000001\n"),
        ("SELECT bin(0) AS value", "value\n00000000\n"),
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
    let query = "SELECT bin()";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
