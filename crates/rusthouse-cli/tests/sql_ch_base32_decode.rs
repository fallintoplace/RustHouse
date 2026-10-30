use std::process::Command;

#[test]
fn verifies_base32_decode_values_and_errors() {
    for (query, expected) in [
        ("SELECT base32Decode('MZXW6===') AS value", "value\nfoo\n"),
        ("SELECT base32Decode('MZXW6YTBOI======') AS value", "value\nfoobar\n"),
        ("SELECT from_base32('MY======') AS value", "value\nf\n"),
        ("SELECT base32Decode('') AS value", "value\n\n"),
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
    let query = "SELECT base32Decode('MZ======')";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
