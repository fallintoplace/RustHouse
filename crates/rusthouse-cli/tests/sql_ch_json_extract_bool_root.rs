use std::process::Command;

fn output(query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap()
}

#[test]
fn extract_bool_supports_root_and_nested_constants() {
    for (sql, expected) in [
        ("SELECT JSONExtractBool('true') AS value", "value\n1\n"),
        ("SELECT JSONExtractBool('false') AS value", "value\n0\n"),
        (
            r#"SELECT JSONExtractBool('{"a":true}', 'a') AS value"#,
            "value\n1\n",
        ),
        (
            r#"SELECT JSONExtractBool('{"a":null}', 'a') AS value"#,
            "value\n0\n",
        ),
        ("SELECT JSONExtractBool('1') AS value", "value\n0\n"),
        ("SELECT JSONExtractBool(NULL) AS value", "value\n\\N\n"),
        (
            "SELECT JSONExtractBool('true', NULL) AS value",
            "value\n\\N\n",
        ),
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
fn extract_bool_rejects_bad_arity() {
    for sql in [
        "SELECT JSONExtractBool()",
        "SELECT JSONExtractBool('true','a','b')",
    ] {
        assert!(!output(sql).status.success(), "{sql}");
    }
}
