use std::process::Command;

#[test]
fn validates_top_level_domain_results() {
    for (query, expected) in [
        ("SELECT topLevelDomain('https://www.clickhouse.com/path') AS value", "value\ncom\n"),
        ("SELECT topLevelDomain('example.co.uk') AS value", "value\nuk\n"),
        ("SELECT topLevelDomain('ftp://user:pass@host.example.org:8080/file') AS value", "value\norg\n"),
        ("SELECT topLevelDomain('localhost') AS value", "value\n\n"),
        ("SELECT topLevelDomain('http://127.0.0.1') AS value", "value\n\n"),
        ("SELECT topLevelDomain('https://bad..example') AS value", "value\n\n"),
        ("SELECT topLevelDomain(NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .expect("execute RustHouse SQL");
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn rejects_top_level_domain_bad_arguments() {
    for query in [
        "SELECT topLevelDomain()",
        "SELECT topLevelDomain('a','b')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
