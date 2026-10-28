use std::process::Command;

#[test]
fn supports_clickhouse_trim_both_constants() {
    for (query, expected) in [
        ("SELECT trimBoth('  x  ') AS value", "value\nx\n"),
        ("SELECT trimBoth('abxyzba', 'ab') AS value", "value\nxyz\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
