use std::process::Command;

#[test]
fn json_type_exposes_nested_types_and_null() {
    for (sql, expected) in [
        (r#"SELECT JSONType('{"a":1}', 'a') AS value"#, "Int64"),
        (r#"SELECT JSONType('{"a":-2}', 'a') AS value"#, "Int64"),
        (r#"SELECT JSONType('{"a":1.5}', 'a') AS value"#, "Double"),
        (r#"SELECT JSONType('{"a":true}', 'a') AS value"#, "Bool"),
        (r#"SELECT JSONType('{"a":"hi"}', 'a') AS value"#, "String"),
        (r#"SELECT JSONType('{"a":[1,2]}', 'a') AS value"#, "Array"),
        (r#"SELECT JSONType('{"a":{}}', 'a') AS value"#, "Object"),
        (r#"SELECT JSONType('{"a":null}', 'a') AS value"#, "Null"),
        (r#"SELECT JSONType('{"a":1}', 'missing') AS value"#, "Null"),
        (
            r#"SELECT JSONType('{"a":{"b":[1]}}', 'a.b') AS value"#,
            "Array",
        ),
        (r#"SELECT JSONType('[1,2]') AS value"#, "Array"),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", sql])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{sql}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            result.stdout,
            format!("value\n{expected}\n").as_bytes(),
            "{sql}"
        );
    }
}

#[test]
fn json_type_validates_arity_and_preserves_sql_null() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT JSONType(NULL) AS value"])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(result.stdout, b"value\n\\N\n");
    for sql in [
        "SELECT JSONType()",
        "SELECT JSONType('{}', 'a', 'b')",
        "SELECT JSONType('{broken')",
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", sql])
            .output()
            .unwrap();
        assert!(!result.status.success(), "{sql}");
    }
}
