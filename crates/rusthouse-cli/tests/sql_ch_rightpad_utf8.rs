use std::process::Command;

#[test]
fn supports_rightpad_utf8_constants() {
    for (query, expected) in [
        ("SELECT rightPadUTF8('é', 3, '*') AS value", "value\né**\n"),
        ("SELECT rightPadUTF8('ab', 4) AS value", "value\nab  \n"),
        ("SELECT rightPadUTF8('猫', 2, '🌟') AS value", "value\n猫🌟\n"),
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
        "SELECT rightPadUTF8('a')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query} unexpectedly succeeded");
    }
}
