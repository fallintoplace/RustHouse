use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn negative_left_offsets_exclude_a_suffix() {
    for (sql, expected) in [
        ("SELECT left('Hello World', -6)", "Hello"),
        ("SELECT left('abc', -1)", "ab"),
        ("SELECT left('abc', -9)", ""),
        ("SELECT left('abc', 0)", ""),
        ("SELECT leftUTF8('Привет', -4)", "Пр"),
        ("SELECT leftUTF8('é🙂Z', -1)", "é🙂"),
        ("SELECT leftUTF8('é🙂Z', 2)", "é🙂"),
        ("SELECT leftUTF8('é🙂Z', -10)", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT leftUTF8(NULL, -1)"), ConstantValue::Null);
    assert!(parse_select("SELECT leftUTF8('abc')").is_err());
    assert!(parse_select("SELECT left('abc', 1, 2)").is_err());
}
