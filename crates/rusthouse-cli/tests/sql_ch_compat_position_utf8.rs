use std::process::Command;

#[test]
fn supports_clickhouse_position_utf8_constants() {
    for (query, expected) in [
        ("SELECT positionUTF8('naïve', 'ï') AS value", "value\n3\n"),
        ("SELECT positionUTF8('abc', 'z') AS value", "value\n0\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
