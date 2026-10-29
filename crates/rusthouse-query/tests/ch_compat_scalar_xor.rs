use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> i64 {
    match parse_select(sql).expect(sql).projection {
        Projection::Constants(values) if values.len() == 1 => match &values[0].value { ConstantValue::Integer(v) => *v, other => panic!("unexpected value: {other:?}"), },
        Projection::Literal(value) => value,
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn boolean_integer_scalar() {
    assert_eq!(value("SELECT xor(1, 0)"), 1);
    assert_eq!(value("SELECT xor(1, 2)"), 0);
}
#[test]
fn rejects_invalid_arity() {
    assert!(parse_select("SELECT xor()").is_err());
    assert!(parse_select("SELECT xor(1)").is_err());
    assert!(parse_select("SELECT xor(1, 2, 3)").is_err());
}
