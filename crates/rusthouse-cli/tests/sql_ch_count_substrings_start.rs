use std::process::Command;

#[test]
fn verifies_count_substrings_start_behavior() {
    for (query, expected) in [
        ("SELECT countSubstrings('banana', 'na', 1) AS value", "value\n2\n"),
        ("SELECT countSubstrings('banana', 'na', 4) AS value", "value\n1\n"),
        ("SELECT countSubstrings('banana', 'na', 99) AS value", "value\n0\n"),
        ("SELECT countSubstrings('banana', 'na', 3) AS value", "value\n1\n"),
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
    let query = "SELECT countSubstrings('banana', 'na', -1)";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
