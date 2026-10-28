use std::process::Command;

#[test]
fn clickhouse_decode_quoted_printable_constants() {
    for (query, expected) in [
        ("SELECT decodeQuotedPrintable('Gr=C3=BC=C3=9Fe') AS value", "value\nGrüße\n"),
        ("SELECT decodeQuotedPrintable('a=3Db') AS value", "value\na=b\n"),
        ("SELECT decodeQuotedPrintable('') AS value", "value\n\n"),
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
