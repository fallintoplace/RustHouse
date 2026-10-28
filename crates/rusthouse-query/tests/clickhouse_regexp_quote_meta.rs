use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn escapes_clickhouse_regex_meta_characters() {
    for (sql, expected) in [
        ("SELECT regexpQuoteMeta('a.b+c')", r"a\.b\+c"),
        ("SELECT regexpQuoteMeta('hello')", "hello"),
        ("SELECT regexpQuoteMeta('a(b)|c')", r"a\(b\)\|c"),
        ("SELECT regexpQuoteMeta('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT regexpQuoteMeta(NULL)"), ConstantValue::Null);
}
#[test]
fn checks_arity_and_input_types() {
    for sql in ["SELECT regexpQuoteMeta()", "SELECT regexpQuoteMeta(1)",
                "SELECT regexpQuoteMeta('a','b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
