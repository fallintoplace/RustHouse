use std::process::Command;

#[test]
fn verifies_base32_encode_values_and_errors() {
    for (query, expected) in [
        ("SELECT base32Encode('foo') AS value", "value\nMZXW6===\n"),
        (
            "SELECT base32Encode('foobar') AS value",
            "value\nMZXW6YTBOI======\n",
        ),
        ("SELECT to_base32('f') AS value", "value\nMY======\n"),
        ("SELECT base32Encode('') AS value", "value\n\n"),
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
    let query = "SELECT base32Encode()";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
