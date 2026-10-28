use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_clickhouse_cases() {
    assert_eq!(value("SELECT positionUTF8('Motörhead', 'r')"), ConstantValue::Integer(5));
    assert_eq!(value("SELECT positionUTF8('éé🙂', '🙂', 2)"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT positionUTF8('aba', 'a', 2)"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT positionUTF8('abc', 'q')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT positionUTF8(NULL, 'a')"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT positionUTF8('abc')",
        "SELECT positionUTF8('abc', 'a', -1)",
        "SELECT positionUTF8('a', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
