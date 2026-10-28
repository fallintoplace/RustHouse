use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn reverses_codepoints_without_splitting_utf8() {
    for (sql, expected) in [
        ("SELECT reverseUTF8('Привет')", "тевирП"),
        ("SELECT reverseUTF8('a🙂é')", "é🙂a"),
        ("SELECT reverseUTF8('')", ""),
        ("SELECT REVERSEUTF8(REVERSE('猫é'))", "猫é"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT reverseUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_argument_types_and_counts() {
    for sql in ["SELECT reverseUTF8()", "SELECT reverseUTF8(2)", "SELECT reverseUTF8('a','b')", "SELECT reverseUTF8('a') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
