use std::process::Command;

#[test]
fn url_port_supports_boundaries() {
    for (query, expected) in [
        ("SELECT port('https://example.com:8443/path') AS value", "value\n8443\n"),
        ("SELECT port('http://user:pass@example.org:8080/') AS value", "value\n8080\n"),
        ("SELECT port('http://example.com') AS value", "value\n0\n"),
        ("SELECT port('http://example.com',443) AS value", "value\n443\n"),
        ("SELECT port('http://[::1]:9000/') AS value", "value\n9000\n"),
        ("SELECT port('http://example.com:99999',443) AS value", "value\n443\n"),
        ("SELECT port(NULL) AS value", "value\n\\N\n"),
        ("SELECT port('http://example.com',NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn url_port_rejects_unsupported_inputs() {
    for query in [
        "SELECT port()",
        "SELECT port('http://example.com',-1)",
        "SELECT port('http://example.com',65536)",
        "SELECT port('http://example.com',80,90)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
