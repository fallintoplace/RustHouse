use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(n) => ConstantValue::Integer(n),
        Projection::Constants(cols) if cols.len() == 1 => cols[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn narrow_integer_or_default() {
    assert_eq!(value("SELECT toUInt16OrDefault('65535')"), ConstantValue::Integer(65535));
    assert_eq!(value("SELECT toUInt16OrDefault('65536')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toUInt16OrDefault('1')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT toUInt16OrDefault('bad', 7)"), ConstantValue::Integer(7));
    assert_eq!(value("SELECT toUInt16OrDefault('bad')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toUInt16OrDefault(NULL)"), ConstantValue::Null);
}
#[test]
fn invalid_arguments() {
    for sql in [
        "SELECT toUInt16OrDefault()",
        "SELECT toUInt16OrDefault('1', 2, 3)",
        "SELECT toUInt16OrDefault('bad', 65536)",
        "SELECT toUInt16OrDefault('1') FROM numbers(2)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
