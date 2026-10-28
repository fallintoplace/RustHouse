use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_clickhouse_spelling_for_utf8_payloads() {
    for (sql, expected) in [
        ("SELECT base64Decode('aGVsbG8=')", "hello"),
        ("SELECT BASE64DECODE('')", ""),
        ("SELECT base64Decode('w6k=')", "é"),
        ("SELECT base64Decode('IQ==')", "!"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT base64Decode(NULL)"), ConstantValue::Null);
}
#[test]
fn preserves_bounds_and_rejects_invalid_base64_or_non_utf8() {
    for sql in ["SELECT base64Decode()", "SELECT base64Decode(1)", "SELECT base64Decode('a','b')", "SELECT base64Decode('bad')", "SELECT base64Decode('/w==')", "SELECT base64Decode('a') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
