use std::process::Command;

#[test]
fn encodes_base32_rfc4648_strings() {
    for (query, expected) in [
        ("SELECT base32Encode('foo') AS value", "value\nMZXW6===\n"),
        ("SELECT base32Encode('foobar') AS value", "value\nMZXW6YTBOI======\n"),
        ("SELECT base32Encode('🦀') AS value", "value\n6CP2NAA=\n"),
        ("SELECT base32Encode(NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
