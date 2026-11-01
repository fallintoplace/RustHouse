use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn negative_right_offsets_exclude_a_prefix() {
    for (sql, expected) in [
        ("SELECT right('Hello World', -6)", "World"),
        ("SELECT right('abc', -1)", "bc"),
        ("SELECT right('abc', -9)", ""),
        ("SELECT right('abc', 0)", ""),
        ("SELECT rightUTF8('Привет', -4)", "ет"),
        ("SELECT rightUTF8('aé🙂Z', -2)", "🙂Z"),
        ("SELECT rightUTF8('aé🙂Z', 2)", "🙂Z"),
        ("SELECT rightUTF8('aé🙂Z', -10)", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT rightUTF8(NULL, -1)"), ConstantValue::Null);
    assert!(parse_select("SELECT rightUTF8('abc')").is_err());
    assert!(parse_select("SELECT right('abc', 1, 2)").is_err());
}
