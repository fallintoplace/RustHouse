use std::process::Command;

#[test]
fn clickhouse_round_on_int64() {
    for (query, expected) in [
        ("SELECT round(125, -1) AS value", "value\n130\n"),
        ("SELECT round(-125, -1) AS value", "value\n-130\n"),
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
