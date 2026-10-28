use std::process::Command;

#[test]
fn supports_clickhouse_base64_encode_constants() {
    for (query, expected) in [
        ("SELECT base64Encode('hi') AS value", "value\naGk=\n"),
        ("SELECT base64Encode('') AS value", "value\n\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
