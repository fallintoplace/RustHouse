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
    assert_eq!(value("SELECT positionCaseInsensitiveUTF8('Привет мир', 'МИР')"), ConstantValue::Integer(8));
    assert_eq!(value("SELECT positionCaseInsensitiveUTF8('École', 'éC')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT positionCaseInsensitiveUTF8('Hi hi', 'HI', 2)"), ConstantValue::Integer(4));
    assert_eq!(value("SELECT positionCaseInsensitiveUTF8('éclair', 'xyz')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT positionCaseInsensitiveUTF8(NULL, 'a')"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT positionCaseInsensitiveUTF8('abc')",
        "SELECT positionCaseInsensitiveUTF8('abc', 'a', -1)",
        "SELECT positionCaseInsensitiveUTF8('a', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
