use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn hex_encode_operates_on_utf8_bytes() {
    assert_eq!(value("SELECT HEX_ENCODE('')"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT HEX_ENCODE('Hi')"), ConstantValue::Text("4869".into()));
    assert_eq!(value("SELECT HEX_ENCODE('é')"), ConstantValue::Text("C3A9".into()));
    assert_eq!(value("SELECT HEX_ENCODE('🙂')"), ConstantValue::Text("F09F9982".into()));
    assert_eq!(value("SELECT HEX_ENCODE(NULL)"), ConstantValue::Null);
}

#[test]
fn hex_encode_rejects_bad_arity_and_large_output() {
    for sql in ["SELECT HEX_ENCODE()", "SELECT HEX_ENCODE('a', 'b')", "SELECT HEX_ENCODE(RPAD('a', 524289, 'a'))"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
