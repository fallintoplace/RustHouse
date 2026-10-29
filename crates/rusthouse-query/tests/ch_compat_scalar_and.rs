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
    assert_eq!(value("SELECT and(2, 3)"), 1);
    assert_eq!(value("SELECT and(0, 3)"), 0);
}
#[test]
fn rejects_invalid_arity() {
    assert!(parse_select("SELECT and()").is_err());
    assert!(parse_select("SELECT and(1)").is_err());
    assert!(parse_select("SELECT and(1, 2, 3)").is_err());
}
