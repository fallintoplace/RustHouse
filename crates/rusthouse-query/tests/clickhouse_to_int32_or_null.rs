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
    assert_eq!(value("SELECT toInt32OrNull('42')"), ConstantValue::Integer(42));
    assert_eq!(value("SELECT toInt32OrNull('-3')"), ConstantValue::Integer(-3));
    assert_eq!(value("SELECT toInt32OrNull('2147483648')"), ConstantValue::Integer(-2147483648));
    assert_eq!(value("SELECT toInt32OrNull('bad')"), ConstantValue::Null);
    assert_eq!(value("SELECT toInt32OrNull(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT toInt32OrNull('1.5')"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT toInt32OrNull()",
        "SELECT toInt32OrNull('1', '2')",
        "SELECT toInt32OrNull('3') FROM numbers(2)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
