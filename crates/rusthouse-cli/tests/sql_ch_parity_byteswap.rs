use std::process::Command;

#[test]
fn clickhouse_byteswap_int64() {
    for (query, expected) in [
        ("SELECT byteSwap(1) AS value", "value\n72057594037927936\n"),
        ("SELECT byteSwap(0) AS value", "value\n0\n"),
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
