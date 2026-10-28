use std::process::Command;

#[test]
fn clickhouse_truncate_on_int64() {
    for (query, expected) in [
        ("SELECT truncate(-129, -1) AS value", "value\n-120\n"),
        ("SELECT truncate(129, -1) AS value", "value\n120\n"),
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
