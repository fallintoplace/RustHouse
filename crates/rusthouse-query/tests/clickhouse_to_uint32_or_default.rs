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
    assert_eq!(value("SELECT toUInt32OrDefault('42')"), ConstantValue::Integer(42));
    assert_eq!(value("SELECT toUInt32OrDefault('4294967295')"), ConstantValue::Integer(4294967295));
    assert_eq!(value("SELECT toUInt32OrDefault('4294967296')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toUInt32OrDefault('bad', 9)"), ConstantValue::Integer(9));
    assert_eq!(value("SELECT toUInt32OrDefault('bad')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toUInt32OrDefault(NULL)"), ConstantValue::Null);
}
#[test]
fn invalid_arguments() {
    for sql in [
        "SELECT toUInt32OrDefault()",
        "SELECT toUInt32OrDefault('1', 2, 3)",
        "SELECT toUInt32OrDefault('bad', -1)",
        "SELECT toUInt32OrDefault('1') FROM numbers(2)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
