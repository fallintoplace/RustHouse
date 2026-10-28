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
    assert_eq!(value("SELECT toInt16OrDefault('32767')"), ConstantValue::Integer(32767));
    assert_eq!(value("SELECT toInt16OrDefault('32768')"), ConstantValue::Integer(-32768));
    assert_eq!(value("SELECT toInt16OrDefault('-1')"), ConstantValue::Integer(-1));
    assert_eq!(value("SELECT toInt16OrDefault('bad', -7)"), ConstantValue::Integer(-7));
    assert_eq!(value("SELECT toInt16OrDefault('bad')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toInt16OrDefault(NULL)"), ConstantValue::Null);
}
#[test]
fn invalid_arguments() {
    for sql in [
        "SELECT toInt16OrDefault()",
        "SELECT toInt16OrDefault('1', 2, 3)",
        "SELECT toInt16OrDefault('bad', 32768)",
        "SELECT toInt16OrDefault('1') FROM numbers(2)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
