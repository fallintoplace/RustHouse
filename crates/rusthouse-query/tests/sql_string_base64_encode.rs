use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn base64_encode_handles_padding_and_utf8() {
    assert_eq!(
        value("SELECT BASE64_ENCODE('')"),
        ConstantValue::Text("".into())
    );
    assert_eq!(
        value("SELECT BASE64_ENCODE('a')"),
        ConstantValue::Text("YQ==".into())
    );
    assert_eq!(
        value("SELECT BASE64_ENCODE('ab')"),
        ConstantValue::Text("YWI=".into())
    );
    assert_eq!(
        value("SELECT BASE64_ENCODE('abc')"),
        ConstantValue::Text("YWJj".into())
    );
    assert_eq!(
        value("SELECT BASE64_ENCODE('hello')"),
        ConstantValue::Text("aGVsbG8=".into())
    );
    assert_eq!(
        value("SELECT BASE64_ENCODE('é')"),
        ConstantValue::Text("w6k=".into())
    );
    assert_eq!(
        value("SELECT BASE64_ENCODE('🙂')"),
        ConstantValue::Text("8J+Zgg==".into())
    );
    assert_eq!(value("SELECT BASE64_ENCODE(NULL)"), ConstantValue::Null);
}

#[test]
fn base64_encode_bounds_result_and_arity() {
    for sql in [
        "SELECT BASE64_ENCODE()",
        "SELECT BASE64_ENCODE('a', 'b')",
        "SELECT BASE64_ENCODE(RPAD('a', 786433, 'a'))",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
