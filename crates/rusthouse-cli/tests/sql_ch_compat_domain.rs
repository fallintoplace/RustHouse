use std::process::Command;

#[test]
fn supports_clickhouse_domain_constants() {
    for (query, expected) in [
        ("SELECT domain('https://user:pass@www.example.com:8443/path?q=1') AS value", "value\nwww.example.com\n"),
        ("SELECT domain('not-a-url') AS value", "value\n\n"),
        ("SELECT domain('https://api.example.org/test') AS value", "value\napi.example.org\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
