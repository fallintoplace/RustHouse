use std::process::Command;

fn sql(query: &str) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .expect("run RustHouse SQL");
    (output.status.success(),
     String::from_utf8(output.stdout).unwrap(),
     String::from_utf8(output.stderr).unwrap())
}

#[test]
fn json_length_counts_arrays_objects_and_nested_values() {
    for (query, expected) in [
        (r#"SELECT JSONLength('[1,2,3]') AS n"#, "n\n3\n"),
        (r#"SELECT JSONLength('{"a":[10,20],"b":{}}') AS n"#, "n\n2\n"),
        (r#"SELECT JSONLength('{"a":[10,20],"b":{}}', 'a') AS n"#, "n\n2\n"),
        (r#"SELECT JSONLength('{"a":[10,20],"b":{}}', 'b') AS n"#, "n\n0\n"),
        (r#"SELECT JSONLength('{"a":[10,20],"b":{}}', 'absent') AS n"#, "n\n0\n"),
        (r#"SELECT JSONLength('{"a":7}', 'a') AS n"#, "n\n0\n"),
        (r#"SELECT JSONLength('null') AS n"#, "n\n0\n"),
        (r#"SELECT JSONLength(NULL) AS n"#, "n\n\\N\n"),
        (r#"SELECT JSONLength('{}', NULL) AS n"#, "n\n\\N\n"),
    ] {
        let (ok, output, error) = sql(query);
        assert!(ok, "{query}: {error}");
        assert_eq!(output, expected, "{query}");
    }
}

#[test]
fn json_length_rejects_invalid_json_and_unsupported_arity() {
    for query in [
        "SELECT JSONLength('{')",
        "SELECT JSONLength('[]', 'a', 'b')",
        "SELECT JSONLength()",
    ] {
        let (ok, _, _) = sql(query);
        assert!(!ok, "accepted {query}");
    }
}
