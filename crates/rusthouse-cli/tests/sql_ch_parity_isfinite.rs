use std::process::Command;

#[test]
fn clickhouse_isfinite_int64() {
    for (query, expected) in [
        ("SELECT isFinite(10) AS value", "value\n1\n"),
        ("SELECT isFinite(-5) AS value", "value\n1\n"),
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
