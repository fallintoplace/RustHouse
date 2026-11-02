use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn replaces_unicode_code_points_with_positive_and_negative_positions() {
    for (sql, expected) in [
        ("SELECT overlayUTF8('café 😊', 'X', 4, 1)", "cafX 😊"),
        ("SELECT overlayUTF8('東京猫', '犬', -1)", "東京犬"),
        ("SELECT overlayUTF8('abcdef', 'Z', -3, 2)", "abcZf"),
        ("SELECT overlayUTF8('abc', 'X', 5)", "abcX"),
        ("SELECT overlayUTF8('abc', 'Z', 1, 0)", "Zabc"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
}

#[test]
fn nulls_and_invalid_arguments_are_bounded() {
    assert_eq!(value("SELECT overlayUTF8(NULL, 'x', 1)"), ConstantValue::Null);
    assert_eq!(value("SELECT overlayUTF8('x', NULL, 1)"), ConstantValue::Null);
    assert_eq!(value("SELECT overlayUTF8('x', 'a', NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT overlayUTF8('x', 'a', 0)",
        "SELECT overlayUTF8('x', 'a', 1, -1)",
        "SELECT overlayUTF8('x', 'a')",
        "SELECT overlayUTF8('x', 'a', 1, 1, 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
