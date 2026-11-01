use std::process::Command;

fn run(query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap()
}

#[test]
fn json_exists_navigates_root_nested_objects_and_arrays() {
    for (sql, expected) in [
        (r#"SELECT JSON_EXISTS('{"a":1}', '$') AS value"#, "1"),
        (r#"SELECT JSON_EXISTS('{"a":1}', '$.a') AS value"#, "1"),
        (r#"SELECT JSON_EXISTS('{"a":null}', '$.a') AS value"#, "1"),
        (
            r#"SELECT JSON_EXISTS('{"a":1}', '$.missing') AS value"#,
            "0",
        ),
        (
            r#"SELECT JSON_EXISTS('{"a":{"b":[{"ok":true}]}}', '$.a.b[0].ok') AS value"#,
            "1",
        ),
        (r#"SELECT JSON_EXISTS('{"a":[1]}', '$.a[1]') AS value"#, "0"),
        (r#"SELECT JSON_EXISTS('{"a":[1]}', '$.a.0') AS value"#, "1"),
    ] {
        let output = run(sql);
        assert!(
            output.status.success(),
            "{sql}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.stdout,
            format!("value\n{expected}\n").as_bytes(),
            "{sql}"
        );
    }
}

#[test]
fn json_exists_rejects_unsupported_path_syntax_and_propagates_null() {
    assert_eq!(
        run("SELECT JSON_EXISTS(NULL, '$') AS value").stdout,
        b"value\n\\N\n"
    );
    for sql in [
        r#"SELECT JSON_EXISTS('{"a":1}', '$..a')"#,
        r#"SELECT JSON_EXISTS('{"a":1}', '$.a[*]')"#,
        r#"SELECT JSON_EXISTS('{bad}', '$.a')"#,
    ] {
        assert!(!run(sql).status.success(), "{sql}");
    }
}
