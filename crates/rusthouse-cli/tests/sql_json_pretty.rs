use std::process::Command;

#[test]
fn pretty_json_is_readable_and_preserves_scalar_values() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT number FROM numbers(2)",
            "--format",
            "json-pretty",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "[\n  {\n    \"number\": \"0\"\n  },\n  {\n    \"number\": \"1\"\n  }\n]\n"
    );

    let empty = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT number FROM numbers(0)",
            "--format",
            "json-pretty",
        ])
        .output()
        .unwrap();
    assert!(empty.status.success());
    assert_eq!(empty.stdout, b"[]\n");
}
