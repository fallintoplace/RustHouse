use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_url_schemes() {
    for (sql, expected) in [
        ("SELECT protocol('https://clickhouse.com/')", "https"),
        ("SELECT protocol('ftp://example.com/file')", "ftp"),
        ("SELECT protocol('mailto:user@example.com')", "mailto"),
        ("SELECT protocol('no-scheme.com/path')", ""),
        ("SELECT protocol('9invalid://host')", ""),
    ] { assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}"); }
    assert_eq!(value("SELECT protocol(NULL)"), ConstantValue::Null);
}
#[test]
fn requires_exactly_one_string() {
    for sql in ["SELECT protocol()", "SELECT protocol(3)", "SELECT protocol('a', 'b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
