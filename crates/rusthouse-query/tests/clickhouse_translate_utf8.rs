use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn maps_multibyte_characters() {
    for (sql, expected) in [
        ("SELECT translateUTF8('Hélló, Wórld¡', 'óé¡', 'oe!')", "Hello, World!"),
        ("SELECT translateUTF8('βaba', 'βa', 'bz')", "bzbz"),
        ("SELECT translateUTF8('', '', '')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT translateUTF8(NULL, 'a','b')"), ConstantValue::Null);
}
#[test]
fn checks_unicode_character_counts_and_arity() {
    for sql in ["SELECT translateUTF8('a','ab','c')", "SELECT translateUTF8('a', 'a')",
                "SELECT translateUTF8('a', 'b', 'c','d')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
