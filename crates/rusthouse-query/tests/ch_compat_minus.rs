use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> i64 {
    match parse_select(sql).expect(sql).projection {
        Projection::Constants(values) if values.len() == 1 => match values[0].value { ConstantValue::Integer(v) => v, ref other => panic!("unexpected value: {other:?}"), },
        Projection::Literal(value) => value,
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn checked_constant_scalar() {
    assert_eq!(value("SELECT minus(5, 3)"), 2);
    assert_eq!(value("SELECT minus(3, 5)"), -2);
}
#[test]
fn rejects_invalid_arity() {
    assert!(parse_select("SELECT minus()").is_err());
    assert!(parse_select("SELECT minus(1)").is_err());
    assert!(parse_select("SELECT minus(1, 2, 3)").is_err());

}
