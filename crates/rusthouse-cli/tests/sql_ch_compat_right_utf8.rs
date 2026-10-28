use std::process::Command;

#[test]
fn supports_clickhouse_right_utf8_constants() {
    for (query, expected) in [
        ("SELECT rightUTF8('rüs', 2) AS value", "value\nüs\n"),
        ("SELECT rightUTF8('abc', 0) AS value", "value\n\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
