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
    assert_eq!(value("SELECT decodeXMLComponent('&lt;x&gt;&amp;&quot;')"), ConstantValue::Text("<x>&\"".into()));
    assert_eq!(value("SELECT decodeXMLComponent('&#65;&#x42;')"), ConstantValue::Text("AB".into()));
    assert_eq!(value("SELECT decodeXMLComponent('&bogus;')"), ConstantValue::Text("&bogus;".into()));
    assert_eq!(value("SELECT decodeXMLComponent('&amp;lt;')"), ConstantValue::Text("&lt;".into()));
    assert_eq!(value("SELECT decodeXMLComponent(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_bad_argument_count_or_type() {
    assert!(parse_select("SELECT decodeXMLComponent()").is_err());
    assert!(parse_select("SELECT decodeXMLComponent(1)").is_err());
    assert!(parse_select("SELECT decodeXMLComponent('a','b')").is_err());
}
