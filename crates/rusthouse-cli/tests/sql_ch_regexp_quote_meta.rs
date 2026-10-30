use std::process::Command;

#[test]
fn supports_regexp_quote_meta_constants() {
    for (query, expected) in [
        ("SELECT regexpQuoteMeta('abc') AS value", "value\nabc\n"),
        ("SELECT regexpQuoteMeta('x.y') AS value", "value\nx\\\\.y\n"),
        ("SELECT regexpQuoteMeta('[a]+') AS value", "value\n\\\\[a\\\\]\\\\+\n"),
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
    let query = "SELECT regexpQuoteMeta()";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
