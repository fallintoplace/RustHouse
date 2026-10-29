use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> i64 {
    match parse_select(sql).expect(sql).projection {
        Projection::Constants(values) if values.len() == 1 => match &values[0].value { ConstantValue::Integer(v) => *v, other => panic!("unexpected value: {other:?}"), },
        Projection::Literal(value) => value,
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn integer_constant_scalar() {
    assert_eq!(value("SELECT isZero(0)"), 1);
    assert_eq!(value("SELECT isZero(-7)"), 0);
}
#[test]
fn invalid_arity_rejected() {
    assert!(parse_select("SELECT isZero()").is_err());
    assert!(parse_select("SELECT isZero(1, 2)").is_err());
    assert!(parse_select("SELECT isZero(1, 2, 3)").is_err());
}
