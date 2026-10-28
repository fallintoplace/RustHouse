use std::process::Command;

#[test]
fn clickhouse_encode_quoted_printable_constants() {
    for (query, expected) in [
        ("SELECT encodeQuotedPrintable('Grüße') AS value", "value\nGr=C3=BC=C3=9Fe\n"),
        ("SELECT encodeQuotedPrintable('a=b') AS value", "value\na=3Db\n"),
        ("SELECT encodeQuotedPrintable('') AS value", "value\n\n"),
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
