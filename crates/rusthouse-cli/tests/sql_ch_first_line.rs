use std::process::Command;

#[test]
fn validates_first_line_results() {
    for (query, expected) in [
        ("SELECT firstLine('hello\nworld') AS value", "value\nhello\n"),
        ("SELECT firstLine('hello\rworld') AS value", "value\nhello\n"),
        ("SELECT firstLine('abc') AS value", "value\nabc\n"),
        ("SELECT firstLine('\nstart') AS value", "value\n\n"),
        ("SELECT firstLine('') AS value", "value\n\n"),
        ("SELECT firstLine(NULL) AS value", "value\n\\N\n"),
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
fn rejects_first_line_bad_arguments() {
    for query in [
        "SELECT firstLine()",
        "SELECT firstLine('one', 'two')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
