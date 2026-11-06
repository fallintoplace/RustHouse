use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn finds_unicode_case_insensitive_positions() {
    for (sql, expected) in [
        ("SELECT positionCaseInsensitiveUTF8('Äpfel', 'äP')", 1),
        (
            "SELECT positionCaseInsensitiveUTF8('Crème brûlée', 'BRÛ')",
            7,
        ),
        ("SELECT positionCaseInsensitiveUTF8('Привет Мир', 'МИР')", 8),
        ("SELECT positionCaseInsensitiveUTF8('abc abc', 'ABC', 4)", 5),
        ("SELECT positionCaseInsensitiveUTF8('abc', 'A', 0)", 1),
        ("SELECT positionCaseInsensitiveUTF8('abc', '', 3)", 3),
        ("SELECT positionCaseInsensitiveUTF8('', '')", 1),
        ("SELECT positionCaseInsensitiveUTF8('abc', '', 9)", 0),
        ("SELECT positionCaseInsensitiveUTF8('abc', 'z')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT positionCaseInsensitiveUTF8(NULL, 'a')"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT positionCaseInsensitiveUTF8('abc', NULL)"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT positionCaseInsensitiveUTF8('a')").is_err());
    assert!(parse_select("SELECT positionCaseInsensitiveUTF8('abc', 'a', -1)").is_err());
    assert!(parse_select("SELECT positionCaseInsensitiveUTF8('a', 'b', 1, 2)").is_err());
}
