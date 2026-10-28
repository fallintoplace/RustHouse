use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn unicode_delimiters_and_signed_counts() {
    for (sql, expected) in [
        ("SELECT substringIndexUTF8('Straße.alpha.beta', '.', 2)", "Straße.alpha"),
        ("SELECT substringIndexUTF8('a•b•c', '•', 1)", "a"),
        ("SELECT substringIndexUTF8('a•b•c', '•', -1)", "c"),
        ("SELECT substringIndexUTF8('no separators', '/', 2)", "no separators"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT substringIndexUTF8(NULL, '.', 1)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_arguments() {
    for sql in ["SELECT substringIndexUTF8()", "SELECT substringIndexUTF8('a', '.')",
                "SELECT substringIndexUTF8(1, '.', 2)", "SELECT substringIndexUTF8('a', '.', 1, 2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
