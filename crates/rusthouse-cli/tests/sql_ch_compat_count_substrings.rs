use std::process::Command;

#[test]
fn supports_clickhouse_count_substrings_constants() {
    for (query, expected) in [
        ("SELECT countSubstrings('banana', 'na') AS value", "value\n2\n"),
        ("SELECT countSubstrings('aaa', 'aa') AS value", "value\n1\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
