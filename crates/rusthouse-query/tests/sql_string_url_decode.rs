use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn url_decode_restores_utf8_and_preserves_plus_signs() {
    assert_eq!(
        value("SELECT URL_DECODE('hello%20world')"),
        ConstantValue::Text("hello world".into())
    );
    assert_eq!(
        value("SELECT URL_DECODE('%C3%A9')"),
        ConstantValue::Text("é".into())
    );
    assert_eq!(
        value("SELECT URL_DECODE('a+b')"),
        ConstantValue::Text("a+b".into())
    );
    assert_eq!(
        value("SELECT URL_DECODE('a%2fb')"),
        ConstantValue::Text("a/b".into())
    );
    assert_eq!(value("SELECT URL_DECODE(NULL)"), ConstantValue::Null);
}

#[test]
fn url_decode_rejects_invalid_escape_sequences() {
    for sql in [
        "SELECT URL_DECODE('%')",
        "SELECT URL_DECODE('%A')",
        "SELECT URL_DECODE('%Q1')",
        "SELECT URL_DECODE('%FF')",
        "SELECT URL_DECODE()",
        "SELECT URL_DECODE('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
