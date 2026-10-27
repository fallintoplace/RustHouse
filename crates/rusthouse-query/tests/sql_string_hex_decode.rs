use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn hex_decode_recovers_utf8_text() {
    assert_eq!(value("SELECT HEX_DECODE('')"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT HEX_DECODE('4869')"), ConstantValue::Text("Hi".into()));
    assert_eq!(value("SELECT HEX_DECODE('c3a9')"), ConstantValue::Text("é".into()));
    assert_eq!(value("SELECT HEX_DECODE('F09F9982')"), ConstantValue::Text("🙂".into()));
    assert_eq!(value("SELECT HEX_DECODE(NULL)"), ConstantValue::Null);
}

#[test]
fn hex_decode_rejects_non_utf8_or_invalid_hex() {
    for sql in ["SELECT HEX_DECODE('a')", "SELECT HEX_DECODE('zz')", "SELECT HEX_DECODE('ff')", "SELECT HEX_DECODE('éé')", "SELECT HEX_DECODE()", "SELECT HEX_DECODE('00', '01')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
