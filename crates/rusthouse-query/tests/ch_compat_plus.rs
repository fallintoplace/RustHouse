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
    assert_eq!(value("SELECT plus(2, 3)"), 5);
    assert_eq!(value("SELECT plus(-3, 2)"), -1);
}
#[test]
fn rejects_invalid_arity_and_overflow() {
    assert!(parse_select("SELECT plus()").is_err());
    assert!(parse_select("SELECT plus(1)").is_err());
    assert!(parse_select("SELECT plus(1, 2, 3)").is_err());
    assert!(parse_select("SELECT plus(9223372036854775807, 1)").is_err());
}
