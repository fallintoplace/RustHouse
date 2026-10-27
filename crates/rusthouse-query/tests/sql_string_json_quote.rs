use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn json_quote_preserves_text_semantics() {
    for (sql, expected) in [
        ("SELECT JSON_QUOTE('hello')", "\"hello\""),
        ("SELECT json_quote('')", "\"\""),
        ("SELECT JSON_QUOTE('a\"b')", "\"a\\\"b\""),
        ("SELECT JSON_QUOTE('it''s')", "\"it's\""),
        ("SELECT JSON_QUOTE('é🙂')", "\"é🙂\""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT JSON_QUOTE(NULL)"), ConstantValue::Null);
}

#[test]
fn json_quote_rejects_invalid_arguments() {
    for sql in [
        "SELECT JSON_QUOTE()",
        "SELECT JSON_QUOTE(1)",
        "SELECT JSON_QUOTE('x','y')",
        "SELECT JSON_QUOTE('x') FROM numbers(1)",
        "SELECT JSON_QUOTE(REPEAT('a', 1048575))",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
