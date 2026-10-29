use std::process::Command;

fn sql(query: &str, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("sql")
        .arg(query)
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn clickhouse_select_format_writes_expected_wire_bytes() {
    for (format, expected) in [
        ("TabSeparated", b"0\n1\n".as_slice()),
        ("TabSeparatedWithNames", b"number\n0\n1\n".as_slice()),
        ("CSV", b"0\r\n1\r\n".as_slice()),
        ("CSVWithNames", b"number\r\n0\r\n1\r\n".as_slice()),
        ("JSONStringsEachRow", b"{\"number\":\"0\"}\n{\"number\":\"1\"}\n".as_slice()),
    ] {
        let query = format!("SELECT number FROM numbers(2) FORMAT {format}");
        let output = sql(&query, &[]);
        assert!(
            output.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected, "{format}");
    }
}

#[test]
fn inline_format_is_not_inferred_from_literals_or_comments() {
    let output = sql("SELECT 'FORMAT CSV' AS value", &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, b"value\nFORMAT CSV\n");
    let output = sql("SELECT number FROM numbers(2) -- FORMAT CSV", &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, b"number\n0\n1\n");
}

#[test]
fn rejects_conflicting_and_unsupported_format_selection() {
    for (query, args) in [
        ("SELECT 1 FORMAT CSV", vec!["--format", "tsv"]),
        ("SELECT 1 FORMAT JSONEachRow", vec![]),
    ] {
        let result = sql(query, &args);
        assert!(!result.status.success(), "{query}");
        assert!(!result.stderr.is_empty());
    }
}
