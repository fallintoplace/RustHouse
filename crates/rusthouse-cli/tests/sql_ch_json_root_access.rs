use std::process::Command;

#[test]
fn json_root_extraction_accepts_one_argument() {
    for (query, expected) in [
        (r#"SELECT JSONHas('{"a":1}') AS value"#, "value\n1\n"),
        (r#"SELECT JSONExtractRaw('{"a":1}') AS value"#, "value\n{\"a\":1}\n"),
        (r#"SELECT JSONExtractString('"hello"') AS value"#, "value\nhello\n"),
        (r#"SELECT JSONExtractString('{"a":1}') AS value"#, "value\n\n"),
        (r#"SELECT JSONExtractRaw('{"a":1}', 'a') AS value"#, "value\n1\n"),
        ("SELECT JSONExtractRaw(NULL) AS value", "value\n\\N\n"),
        (r#"SELECT JSONExtractRaw('{"a":1}', NULL) AS value"#, "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse")).args(["sql", query]).output().unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn json_root_access_requires_at_least_one_argument() {
    for sql in ["SELECT JSONHas()", "SELECT JSONExtractRaw('{}', 'a', 'b')"] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse")).args(["sql", sql]).output().unwrap();
        assert!(!result.status.success(), "{sql}");
    }
}
