use std::process::Command;

#[test]
fn clickhouse_roundbankers_on_int64() {
    for (query, expected) in [
        ("SELECT roundBankers(125, -1) AS value", "value\n120\n"),
        ("SELECT roundBankers(135, -1) AS value", "value\n140\n"),
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
