use std::process::Command;

#[test]
fn supports_clickhouse_replace_one_constants() {
    for (query, expected) in [
        ("SELECT replaceOne('banana', 'na', 'X') AS value", "value\nbaXna\n"),
        ("SELECT replaceOne('abc', 'z', 'x') AS value", "value\nabc\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
