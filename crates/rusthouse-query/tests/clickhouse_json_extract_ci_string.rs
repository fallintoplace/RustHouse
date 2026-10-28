use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn json_extract_string_case_insensitive_supports_keys_and_indices() {
    for (sql, expected) in [
        (r#"SELECT JSONExtractStringCaseInsensitive('{"ABC":"def"}', 'abc')"#, "def"),
        (r#"SELECT JSONExtractStringCaseInsensitive('{"User":{"Name":"John"}}','user','name')"#, "John"),
        (r#"SELECT JSONExtractStringCaseInsensitive('{"items":["a","b"]}','ITEMS',2)"#, "b"),
        (r#"SELECT JSONExtractStringCaseInsensitive('{"items":["a","b"]}','ITEMS',-1)"#, "b"),
        (r#"SELECT JSONExtractStringCaseInsensitive('{"x":8}','X')"#, ""),
        (r#"SELECT JSONExtractStringCaseInsensitive('{"x":"yes"}','missing')"#, ""),
        (r#"SELECT JSONExtractStringCaseInsensitive('"text"')"#, "text"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT JSONExtractStringCaseInsensitive(NULL,'x')"), ConstantValue::Null);
}
#[test]
fn json_extract_string_case_insensitive_rejects_invalid_inputs() {
    for sql in [
        "SELECT JSONExtractStringCaseInsensitive('broken', 'a')",
        "SELECT JSONExtractStringCaseInsensitive('{}', 'a', 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
