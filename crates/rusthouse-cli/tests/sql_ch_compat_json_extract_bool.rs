use std::process::Command;

#[test]
fn supports_clickhouse_json_extract_bool_constants() {
    for (query, expected) in [
        ("SELECT JSONExtractBool('{\"active\":true}', 'active') AS value", "value\n1\n"),
        ("SELECT JSONExtractBool('{\"active\":false}', 'active') AS value", "value\n0\n"),
        ("SELECT JSONExtractBool('{\"active\":true}', 'missing') AS value", "value\n0\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
