use std::process::Command;

#[test]
fn supports_startswith_utf8_constants() {
    for (query, expected) in [
        ("SELECT startsWithUTF8('猫咪', '猫') AS value", "value\n1\n"),
        ("SELECT startsWithUTF8('éclair', 'é') AS value", "value\n1\n"),
        ("SELECT startsWithUTF8('éclair', 'e') AS value", "value\n0\n"),
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
    for query in [
        "SELECT startsWithUTF8('text')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query} unexpectedly succeeded");
    }
}
