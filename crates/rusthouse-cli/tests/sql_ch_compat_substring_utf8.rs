use std::process::Command;

#[test]
fn supports_clickhouse_substring_utf8_constants() {
    for (query, expected) in [
        ("SELECT substringUTF8('naïve', 3, 2) AS value", "value\nïv\n"),
        ("SELECT substringUTF8('abc', 2) AS value", "value\nbc\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
