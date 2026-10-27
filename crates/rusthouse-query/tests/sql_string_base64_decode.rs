use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn base64_decode_handles_padding_and_unicode() {
    assert_eq!(value("SELECT BASE64_DECODE('')"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT BASE64_DECODE('YQ==')"), ConstantValue::Text("a".into()));
    assert_eq!(value("SELECT BASE64_DECODE('YWI=')"), ConstantValue::Text("ab".into()));
    assert_eq!(value("SELECT BASE64_DECODE('YWJj')"), ConstantValue::Text("abc".into()));
    assert_eq!(value("SELECT BASE64_DECODE('aGVsbG8=')"), ConstantValue::Text("hello".into()));
    assert_eq!(value("SELECT BASE64_DECODE('w6k=')"), ConstantValue::Text("é".into()));
    assert_eq!(value("SELECT BASE64_DECODE('8J+Zgg==')"), ConstantValue::Text("🙂".into()));
    assert_eq!(value("SELECT BASE64_DECODE(NULL)"), ConstantValue::Null);
}

#[test]
fn base64_decode_rejects_invalid_encoding() {
    for sql in ["SELECT BASE64_DECODE('YQ=')", "SELECT BASE64_DECODE('YQ=A')", "SELECT BASE64_DECODE('Yf==')", "SELECT BASE64_DECODE('YQ==YQ==')", "SELECT BASE64_DECODE('!!!!')", "SELECT BASE64_DECODE('//8=')", "SELECT BASE64_DECODE()", "SELECT BASE64_DECODE('a', 'b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
