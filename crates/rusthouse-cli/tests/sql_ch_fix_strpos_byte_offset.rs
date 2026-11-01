use std::process::Command;

#[test]
fn verifies_strpos_byte_offset_semantics() {
    for (query, expected) in [
        ("SELECT strpos('éabc', 'a') AS value", "value\n3\n"),
        ("SELECT positionUTF8('éabc', 'a') AS value", "value\n2\n"),
        ("SELECT strpos('hello', 'll') AS value", "value\n3\n"),
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
}
