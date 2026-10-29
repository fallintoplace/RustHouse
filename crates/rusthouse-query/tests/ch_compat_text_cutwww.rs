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
    assert_eq!(value("SELECT cutWWW('https://www.example.com/p')"), ConstantValue::Text("https://example.com/p".into()));
    assert_eq!(value("SELECT cutWWW('www.example.com/path')"), ConstantValue::Text("example.com/path".into()));
    assert_eq!(value("SELECT cutWWW('http://example.com')"), ConstantValue::Text("http://example.com".into()));
}
#[test]
fn rejects_wrong_arguments() {
    assert!(parse_select("SELECT cutWWW()").is_err());
    assert!(parse_select("SELECT cutWWW(1)").is_err());
    assert!(parse_select("SELECT cutWWW('a', 'b')").is_err());
}
