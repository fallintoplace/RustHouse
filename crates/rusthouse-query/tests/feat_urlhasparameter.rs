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
        ("SELECT urlHasParameter('https://example.test/?a=1&b=', 'b')", 1),
        ("SELECT urlHasParameter('https://example.test/?a=1', 'b')", 0),
        ("SELECT urlHasParameter('https://example.test/#?a=1', 'a')", 0),
        ("SELECT urlHasParameter('https://example.test/?b=2', '')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT urlHasParameter(NULL, 'a')"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arity() {
    for sql in [
        "SELECT urlHasParameter('x')",
        "SELECT urlHasParameter('x', 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
