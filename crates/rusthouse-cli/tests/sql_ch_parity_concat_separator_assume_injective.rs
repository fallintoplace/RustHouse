use std::process::Command;

#[test]
fn clickhouse_concat_separator_assume_injective_constants() {
    for (query, expected) in [
        ("SELECT concatWithSeparatorAssumeInjective('-', 'a', 'b') AS value", "value\na-b\n"),
        ("SELECT concatWithSeparatorAssumeInjective(':', 'x', 'y') AS value", "value\nx:y\n"),
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
