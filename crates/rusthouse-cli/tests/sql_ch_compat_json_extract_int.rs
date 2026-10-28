use std::process::Command;

#[test]
fn supports_clickhouse_json_extract_int_constants() {
    for (query, expected) in [
        ("SELECT JSONExtractInt('{\"n\":-42}', 'n') AS value", "value\n-42\n"),
        ("SELECT JSONExtractInt('{\"n\":1}', 'missing') AS value", "value\n0\n"),
        ("SELECT JSONExtractInt('{\"items\":[{\"n\":7}]}', 'items.0.n') AS value", "value\n7\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
