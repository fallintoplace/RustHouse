use std::process::Command;

fn sql(query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query]).output().unwrap()
}

#[test]
fn signed_casts_accept_strings_numbers_and_nested_constants() {
    for (query, value) in [
        ("SELECT toInt8('127') AS value", "127"),
        ("SELECT toInt8(128) AS value", "-128"),
        ("SELECT toInt8(-129) AS value", "127"),
        ("SELECT toInt16(65535) AS value", "-1"),
        ("SELECT toInt32('2147483647') AS value", "2147483647"),
        ("SELECT toInt64('-9223372036854775808') AS value", "-9223372036854775808"),
        ("SELECT toInt8(toInt16(130)) AS value", "-126"),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {}", String::from_utf8_lossy(&result.stderr));
        assert_eq!(result.stdout, format!("value\n{value}\n").as_bytes(), "{query}");
    }
}

#[test]
fn signed_casts_reject_bad_strings_and_propagate_null() {
    assert_eq!(sql("SELECT toInt16(NULL) AS value").stdout, b"value\n\\N\n");
    for query in [
        "SELECT toInt8('nan')",
        "SELECT toInt64('9.8')",
        "SELECT toInt16()",
        "SELECT toInt32(3, 4)",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
}
