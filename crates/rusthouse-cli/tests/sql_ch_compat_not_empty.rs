use std::process::Command;

#[test]
fn supports_clickhouse_not_empty_constants() {
    for (query, expected) in [
        ("SELECT notEmpty('') AS value", "value\n0\n"),
        ("SELECT notEmpty('x') AS value", "value\n1\n"),
        ("SELECT notEmpty(NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
