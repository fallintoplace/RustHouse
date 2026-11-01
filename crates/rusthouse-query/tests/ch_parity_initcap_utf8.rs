use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn capitalizes_unicode_word_starts() {
    for (sql, expected) in [
        ("SELECT initcapUTF8('не тормозит')", "Не Тормозит"),
        ("SELECT initcapUTF8('mÜNCHEN & пРИВЕТ')", "München & Привет"),
        ("SELECT initcapUTF8('john''s cat')", "John'S Cat"),
        ("SELECT initcapUTF8('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT initcapUTF8(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT initcapUTF8(1)").is_err());
    assert!(parse_select("SELECT initcapUTF8('x', 'y')").is_err());
}
