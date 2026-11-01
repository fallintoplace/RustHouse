use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn uses_bytes_for_right_pad_and_characters_for_utf8_variant() {
    for (sql, expected) in [
        ("SELECT rightPad('é', 3, 'x')", "éx"),
        ("SELECT rightPadUTF8('é', 3, 'x')", "éxx"),
        ("SELECT rightPad('é猫', 6, 'x')", "é猫x"),
        ("SELECT rightPad('a', 5, 'xy')", "axyxy"),
        ("SELECT rightPadUTF8('é🙂', 3, 'x')", "é🙂x"),
        ("SELECT rightPad('abcdef', 3)", "abc"),
        ("SELECT rightPadUTF8('é猫', 1)", "é"),
        ("SELECT rightPad('a', 5, 'é')", "aéé"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT rightPad(NULL, 2)"), ConstantValue::Null);
}

#[test]
fn rejects_byte_widths_that_split_utf8() {
    assert!(parse_select("SELECT rightPad('é', 1)").is_err());
    assert!(parse_select("SELECT rightPad('a', 4, 'é')").is_err());
}
