use std::process::Command;

#[test]
fn supports_clickhouse_to_int64_or_null_constants() {
    for (query, expected) in [
        ("SELECT toInt64OrNull('42') AS value", "value\n42\n"),
        ("SELECT toInt64OrNull('invalid') AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
