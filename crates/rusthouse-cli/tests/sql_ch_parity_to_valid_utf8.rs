use std::process::Command;

#[test]
fn clickhouse_to_valid_utf8_constants() {
    for (query, expected) in [
        ("SELECT toValidUTF8('München') AS value", "value\nMünchen\n"),
        ("SELECT toValidUTF8('') AS value", "value\n\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .expect("run rusthouse");
        assert!(
            output.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
