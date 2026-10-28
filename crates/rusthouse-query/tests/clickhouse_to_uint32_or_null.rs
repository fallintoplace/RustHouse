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
    assert_eq!(value("SELECT toUInt32OrNull('42')"), ConstantValue::Integer(42));
    assert_eq!(value("SELECT toUInt32OrNull('4294967295')"), ConstantValue::Integer(4294967295));
    assert_eq!(value("SELECT toUInt32OrNull('4294967296')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toUInt32OrNull('bad')"), ConstantValue::Null);
    assert_eq!(value("SELECT toUInt32OrNull(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT toUInt32OrNull('1.5')"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT toUInt32OrNull()",
        "SELECT toUInt32OrNull('1', '2')",
        "SELECT toUInt32OrNull('3') FROM numbers(2)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
