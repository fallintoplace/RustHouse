use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(number) => ConstantValue::Integer(number),
        Projection::Constants(cols) if cols.len() == 1 => cols[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn converts_strings_with_clickhouse_bounded_semantics() {
    assert_eq!(value("SELECT toInt32OrDefault('42')"), ConstantValue::Integer(42));
    assert_eq!(value("SELECT toInt32OrDefault('bad')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toInt32OrDefault('bad', -7)"), ConstantValue::Integer(-7));
    assert_eq!(value("SELECT toInt32OrDefault('2147483648')"), ConstantValue::Integer(-2147483648));
    assert_eq!(value("SELECT toInt32OrDefault('bad')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toInt32OrDefault(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT toInt32OrDefault('1.5')"), ConstantValue::Integer(0));
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT toInt32OrDefault()",
        "SELECT toInt32OrDefault('1', 2, 3)",
        "SELECT toInt32OrDefault('no', 2147483648)",
        "SELECT toInt32OrDefault('3') FROM numbers(2)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
