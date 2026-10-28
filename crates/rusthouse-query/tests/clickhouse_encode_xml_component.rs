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
    assert_eq!(value("SELECT encodeXMLComponent('<a>&\"')"), ConstantValue::Text("&lt;a&gt;&amp;&quot;".into()));
    assert_eq!(value("SELECT encodeXMLComponent('plain')"), ConstantValue::Text("plain".into()));
    assert_eq!(value("SELECT encodeXMLComponent('O''Brien')"), ConstantValue::Text("O&apos;Brien".into()));
    assert_eq!(value("SELECT encodeXMLComponent(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_bad_argument_count_or_type() {
    assert!(parse_select("SELECT encodeXMLComponent()").is_err());
    assert!(parse_select("SELECT encodeXMLComponent(1)").is_err());
    assert!(parse_select("SELECT encodeXMLComponent('a','b')").is_err());
}
