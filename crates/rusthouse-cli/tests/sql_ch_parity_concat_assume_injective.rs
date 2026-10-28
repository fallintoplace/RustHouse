use std::process::Command;

#[test]
fn clickhouse_concat_assume_injective_constants() {
    for (query, expected) in [
        ("SELECT concatAssumeInjective('Rust', 'House') AS value", "value\nRustHouse\n"),
        ("SELECT concatAssumeInjective('a', NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .expect("start rusthouse");
        assert!(
            output.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
