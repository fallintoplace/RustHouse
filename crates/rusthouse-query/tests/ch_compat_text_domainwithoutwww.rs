use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).expect(sql).projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn clickhouse_constant_compatibility() {
    assert_eq!(value("SELECT domainWithoutWWW('https://www.example.com/p?a=1')"), ConstantValue::Text("example.com".into()));
    assert_eq!(value("SELECT domainWithoutWWW('https://example.com/p')"), ConstantValue::Text("example.com".into()));
    assert_eq!(value("SELECT domainWithoutWWW('not-a-url')"), ConstantValue::Text("".into()));
}
#[test]
fn rejects_wrong_arguments() {
    assert!(parse_select("SELECT domainWithoutWWW()").is_err());
    assert!(parse_select("SELECT domainWithoutWWW(1)").is_err());
    assert!(parse_select("SELECT domainWithoutWWW('a', 'b')").is_err());
}
