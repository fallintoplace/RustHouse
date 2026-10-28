use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn clickhouse_parity_examples() {
    assert_eq!(value("SELECT firstLine('foo\nbar')"), ConstantValue::Text("foo".into()), "SELECT firstLine('foo\nbar')");
    assert_eq!(value("SELECT firstLine('single')"), ConstantValue::Text("single".into()), "SELECT firstLine('single')");
    assert_eq!(value("SELECT firstLine('')"), ConstantValue::Text("".into()), "SELECT firstLine('')");
    assert_eq!(value("SELECT firstline(NULL)"), ConstantValue::Null);
}
#[test]
fn invalid_arity_and_type_are_rejected() {
    assert!(parse_select("SELECT firstLine()").is_err());
    assert!(parse_select("SELECT firstLine(3)").is_err());
    assert!(parse_select("SELECT firstLine('a','b')").is_err());
}
