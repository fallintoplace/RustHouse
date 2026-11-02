use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn decodes_valid_url_components_without_treating_plus_as_space() {
    for (sql, expected) in [
        ("SELECT tryDecodeURLComponent('hello%20world')", "hello world"),
        ("SELECT tryDecodeURLComponent('caf%C3%A9')", "café"),
        ("SELECT tryDecodeURLComponent('a+b')", "a+b"),
        ("SELECT tryDecodeURLComponent('%25')", "%"),
        ("SELECT tryDecodeURLComponent('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
}

#[test]
fn malformed_percent_encodings_return_null_instead_of_failing() {
    for sql in [
        "SELECT tryDecodeURLComponent('%')",
        "SELECT tryDecodeURLComponent('%2')",
        "SELECT tryDecodeURLComponent('%xy')",
        "SELECT tryDecodeURLComponent('%FF')",
    ] {
        assert_eq!(value(sql), ConstantValue::Null, "{sql}");
    }
    assert_eq!(value("SELECT tryDecodeURLComponent(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT tryDecodeURLComponent()").is_err());
    assert!(parse_select("SELECT tryDecodeURLComponent('ok', 1)").is_err());
    assert!(parse_select("SELECT decodeURLComponent('%')").is_err());
}
