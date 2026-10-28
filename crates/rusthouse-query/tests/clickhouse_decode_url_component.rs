use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn clickhouse_decode_url_component_handles_percent_escapes() {
    assert_eq!(value("SELECT decodeURLComponent('a%20b%2Bc')"), ConstantValue::Text("a b+c".into()));
    assert_eq!(value("SELECT decodeURLComponent('a+b')"), ConstantValue::Text("a+b".into()));
    assert_eq!(value("SELECT decodeURLComponent(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT decodeURLComponent(URL_ENCODE('Grüße'))"), ConstantValue::Text("Grüße".into()));
}
#[test]
fn clickhouse_decode_url_component_rejects_bad_arguments() {
    for sql in [
        "SELECT decodeURLComponent()",
        "SELECT decodeURLComponent(1)",
        "SELECT decodeURLComponent('abc', 'def')",
        "SELECT decodeURLComponent('%GG')",
        "SELECT decodeURLComponent('abc') FROM numbers(1)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
