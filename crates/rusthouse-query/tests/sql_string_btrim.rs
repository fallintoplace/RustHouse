use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn btrim_supports_custom_unicode_character_sets() {
    for (sql, expected) in [
        ("SELECT BTRIM('  hello  ')", "hello"),
        ("SELECT btrim('xxhelloXY', 'xXY')", "hello"),
        ("SELECT BTRIM('🙂éhelloé🙂', 'é🙂')", "hello"),
        ("SELECT BTRIM('abba', 'ab')", ""),
        ("SELECT BTRIM('hello', '')", "hello"),
        ("SELECT BTRIM(LOWER('  HI  '))", "hi"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT BTRIM(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT BTRIM('x', NULL)"), ConstantValue::Null);
}

#[test]
fn btrim_rejects_invalid_types_and_arity() {
    for sql in [
        "SELECT BTRIM()",
        "SELECT BTRIM(7)",
        "SELECT BTRIM('x', 1)",
        "SELECT BTRIM('x', 'x', 'y')",
        "SELECT BTRIM('x') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
