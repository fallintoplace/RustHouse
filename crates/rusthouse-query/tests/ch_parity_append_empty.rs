use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn append_trailing_char_skips_empty_input() {
    for (sql, expected) in [
        ("SELECT appendTrailingCharIfAbsent('', '/')", ""),
        ("SELECT appendTrailingCharIfAbsent('abc', '/')", "abc/"),
        ("SELECT appendTrailingCharIfAbsent('abc/', '/')", "abc/"),
        ("SELECT appendTrailingCharIfAbsent('é', '/')", "é/"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT appendTrailingCharIfAbsent(NULL, '/')"),
        ConstantValue::Null
    );
}

#[test]
fn append_trailing_char_rejects_invalid_arity() {
    assert!(parse_select("SELECT appendTrailingCharIfAbsent('x')").is_err());
    assert!(parse_select("SELECT appendTrailingCharIfAbsent('x', 'ab')").is_err());
}
