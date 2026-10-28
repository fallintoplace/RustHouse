use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_clickhouse_spelling_and_utf8() {
    for (sql, expected) in [
        ("SELECT base64Encode('hello')", "aGVsbG8="),
        ("SELECT BASE64ENCODE('')", ""),
        ("SELECT base64Encode('é')", "w6k="),
        ("SELECT base64Encode('!')", "IQ=="),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT base64Encode(NULL)"), ConstantValue::Null);
}
#[test]
fn validates_inputs() {
    for sql in ["SELECT base64Encode()", "SELECT base64Encode(1)", "SELECT base64Encode('a','b')", "SELECT base64Encode('a') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
