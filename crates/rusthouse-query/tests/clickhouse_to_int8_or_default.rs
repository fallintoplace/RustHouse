use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(n) => ConstantValue::Integer(n),
        Projection::Constants(cols) if cols.len() == 1 => cols[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn conversion_and_default() {
    assert_eq!(value("SELECT toInt8OrDefault('127')"), ConstantValue::Integer(127));
    assert_eq!(value("SELECT toInt8OrDefault('128')"), ConstantValue::Integer(-128));
    assert_eq!(value("SELECT toInt8OrDefault('-1')"), ConstantValue::Integer(-1));
    assert_eq!(value("SELECT toInt8OrDefault('bad', -7)"), ConstantValue::Integer(-7));
    assert_eq!(value("SELECT toInt8OrDefault('bad')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toInt8OrDefault(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_bad_arity_and_default() {
    for sql in [
        "SELECT toInt8OrDefault()",
        "SELECT toInt8OrDefault('bad', 128)",
        "SELECT toInt8OrDefault('1', 2, 3)",
        "SELECT toInt8OrDefault('1') FROM numbers(2)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
