use std::process::Command;

#[test]
fn clickhouse_ceil_integer_rounding() {
    for (query, expected) in [
        ("SELECT ceil(121, -1) AS value", "value\n130\n"),
        ("SELECT ceil(-121, -1) AS value", "value\n-120\n"),
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
