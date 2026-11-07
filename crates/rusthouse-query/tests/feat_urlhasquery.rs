use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn url_query_detection_examples() {
    for (sql, expected) in [
        ("SELECT urlHasQuery('https://example.test/?')", 1),
        ("SELECT urlHasQuery('https://example.test/?a=1')", 1),
        ("SELECT urlHasQuery('https://example.test/path')", 0),
        ("SELECT urlHasQuery('https://example.test/#?a=1')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT urlHasQuery(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arity() {
    for sql in [
        "SELECT urlHasQuery()",
        "SELECT urlHasQuery('x', 'y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
