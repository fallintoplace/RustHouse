use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn covers_clickhouse_expression_cases() {
    assert_eq!(value("SELECT startsWithUTF8('Привет','При')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT startsWithUTF8('Привет','вет')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT startsWithUTF8('foo','')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT startsWithUTF8('é','ê')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT startsWithUTF8(NULL,'a')"), ConstantValue::Null);
}
#[test]
fn rejects_bad_argument_count_or_type() {
    assert!(parse_select("SELECT startsWithUTF8()").is_err());
    assert!(parse_select("SELECT startsWithUTF8('a')").is_err());
    assert!(parse_select("SELECT startsWithUTF8('a','b','c')").is_err());
    assert!(parse_select("SELECT startsWithUTF8(1,'a')").is_err());
}
