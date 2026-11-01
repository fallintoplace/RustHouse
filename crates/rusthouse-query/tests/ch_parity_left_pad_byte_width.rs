use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn uses_bytes_for_left_pad_and_characters_for_utf8_variant() {
    for (sql, expected) in [
        ("SELECT leftPad('é', 3, 'x')", "xé"),
        ("SELECT leftPadUTF8('é', 3, 'x')", "xxé"),
        ("SELECT leftPad('é猫', 6, 'x')", "xé猫"),
        ("SELECT leftPad('a', 5, 'xy')", "xyxya"),
        ("SELECT leftPadUTF8('é🙂', 3, 'x')", "xé🙂"),
        ("SELECT leftPad('abcdef', 3)", "abc"),
        ("SELECT leftPadUTF8('é猫', 1)", "é"),
        ("SELECT leftPad('a', 5, 'é')", "ééa"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT leftPad(NULL, 2)"), ConstantValue::Null);
}

#[test]
fn rejects_byte_widths_that_split_utf8() {
    assert!(parse_select("SELECT leftPad('é', 1)").is_err());
    assert!(parse_select("SELECT leftPad('a', 4, 'é')").is_err());
}
