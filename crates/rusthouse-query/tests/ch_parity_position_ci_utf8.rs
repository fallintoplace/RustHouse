use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn finds_case_insensitive_unicode_positions_in_code_points() {
    for (sql, expected) in [
        ("SELECT positionCaseInsensitiveUTF8('ЖУК жук', 'жук')", 1),
        ("SELECT positionCaseInsensitiveUTF8('ЖУК жук', 'жук', 2)", 5),
        ("SELECT positionCaseInsensitiveUTF8('ééC', 'c')", 3),
        ("SELECT positionCaseInsensitiveUTF8('Äbc ÄBC', 'äbc', 2)", 5),
        ("SELECT positionCaseInsensitiveUTF8('abc', 'x')", 0),
        ("SELECT positionCaseInsensitiveUTF8('abc', '')", 1),
        ("SELECT positionCaseInsensitive('Äbc', 'ä')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT positionCaseInsensitiveUTF8(NULL, 'a')"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT positionCaseInsensitiveUTF8('abc')").is_err());
    assert!(parse_select("SELECT positionCaseInsensitiveUTF8('abc', 'a', -1)").is_err());
}
