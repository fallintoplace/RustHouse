use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn compares_case_insensitive_prefixes_and_suffixes() {
    assert_eq!(value("SELECT startsWithCaseInsensitive('ClickHouse','click')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT startsWithCaseInsensitive('ClickHouse','house')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT startsWithCaseInsensitive('abc','')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT startsWithCaseInsensitive('a','abcdef')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT startswithcaseinsensitive(NULL, 'x')"), ConstantValue::Null);
    assert_eq!(value("SELECT startswithcaseinsensitive('x', NULL)"), ConstantValue::Null);
}
#[test]
fn validates_argument_counts() {
    for sql in ["SELECT startswithcaseinsensitive()", "SELECT startswithcaseinsensitive('x')", "SELECT startswithcaseinsensitive(1,'x')", "SELECT startswithcaseinsensitive('x','y','z')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
