use std::process::Command;

#[test]
fn clickhouse_isinfinite_for_int64() {
    for (query, expected) in [
        ("SELECT ISINFINITE(0) AS value", "value\n0\n"),
        ("SELECT ISINFINITE(-123) AS value", "value\n0\n"),
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
