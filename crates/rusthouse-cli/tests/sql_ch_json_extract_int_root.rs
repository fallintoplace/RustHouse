use std::process::Command;

fn output(query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap()
}

#[test]
fn extract_int_supports_root_and_nested_constants() {
    for (sql, expected) in [
        ("SELECT JSONExtractInt('42') AS value", "value\n42\n"),
        ("SELECT JSONExtractInt('-7') AS value", "value\n-7\n"),
        (
            r#"SELECT JSONExtractInt('{"a":12}', 'a') AS value"#,
            "value\n12\n",
        ),
        (
            r#"SELECT JSONExtractInt('{"a":"12"}', 'a') AS value"#,
            "value\n0\n",
        ),
        ("SELECT JSONExtractInt('true') AS value", "value\n0\n"),
        ("SELECT JSONExtractInt(NULL) AS value", "value\n\\N\n"),
        ("SELECT JSONExtractInt('1', NULL) AS value", "value\n\\N\n"),
    ] {
        let result = output(sql);
        assert!(
            result.status.success(),
            "{sql}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, expected.as_bytes(), "{sql}");
    }
}

#[test]
fn extract_int_rejects_bad_arity() {
    for sql in [
        "SELECT JSONExtractInt()",
        "SELECT JSONExtractInt('1','a','b')",
    ] {
        assert!(!output(sql).status.success(), "{sql}");
    }
}
