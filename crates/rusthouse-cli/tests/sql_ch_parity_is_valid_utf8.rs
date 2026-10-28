use std::process::Command;

#[test]
fn clickhouse_is_valid_utf8_constants() {
    for (query, expected) in [
        ("SELECT isValidUTF8('Grüße') AS value", "value\n1\n"),
        ("SELECT isValidUTF8('') AS value", "value\n1\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .expect("start rusthouse");
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
