use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn clickhouse_concat_with_separator_values() {
    assert_eq!(value("SELECT concatWithSeparator('-', 'a', 'b', 'c')"), ConstantValue::Text("a-b-c".into()));
    assert_eq!(value("SELECT concatWithSeparator('/', 'A', 'B')"), ConstantValue::Text("A/B".into()));
    assert_eq!(value("SELECT concatWithSeparator(NULL,'a')"), ConstantValue::Null);
}
#[test]
fn clickhouse_concat_with_separator_invalid_arguments() {
    for sql in [
        "SELECT concatWithSeparator()",
        "SELECT concatWithSeparator(3, 'x')",
        "SELECT concatWithSeparator('-',1)",
        "SELECT concatWithSeparator('a') FROM numbers(1)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
