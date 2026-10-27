use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn url_encode_percent_encodes_utf8_bytes() {
    assert_eq!(value("SELECT URL_ENCODE('hello world')"), ConstantValue::Text("hello%20world".into()));
    assert_eq!(value("SELECT URL_ENCODE('a/b?c=d')"), ConstantValue::Text("a%2Fb%3Fc%3Dd".into()));
    assert_eq!(value("SELECT URL_ENCODE('é')"), ConstantValue::Text("%C3%A9".into()));
    assert_eq!(value("SELECT URL_ENCODE('AZaz09-._~')"), ConstantValue::Text("AZaz09-._~".into()));
    assert_eq!(value("SELECT URL_ENCODE(NULL)"), ConstantValue::Null);
}

#[test]
fn url_encode_limits_output_and_checks_arity() {
    for sql in ["SELECT URL_ENCODE()", "SELECT URL_ENCODE('a', 'b')", "SELECT URL_ENCODE(SPACE(400000))"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
