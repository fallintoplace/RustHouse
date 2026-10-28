use std::process::Command;

#[test]
fn clickhouse_intexp2_int64() {
    for (query, expected) in [
        ("SELECT intExp2(10) AS value", "value\n1024\n"),
        ("SELECT intExp2(0) AS value", "value\n1\n"),
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
