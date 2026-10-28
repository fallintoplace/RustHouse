use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn position_is_one_based_and_ascii_insensitive() {
    for (sql, expected) in [
        ("SELECT positionCaseInsensitive('Hello World', 'WORLD')", 7),
        ("SELECT positionCaseInsensitive('AaAa', 'aa', 3)", 3),
        ("SELECT positionCaseInsensitive('abc', 'ZZ')", 0),
        ("SELECT positionCaseInsensitive('abc', '')", 1),
        ("SELECT positionCaseInsensitive('abc', '', 4)", 4),
        ("SELECT positionCaseInsensitive('ÄBc', 'bc')", 3),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT positionCaseInsensitive(NULL, 'a')"),
        ConstantValue::Null
    );
}
#[test]
fn rejects_invalid_position_and_type() {
    for sql in [
        "SELECT positionCaseInsensitive('a')",
        "SELECT positionCaseInsensitive('a', 'a', -1)",
        "SELECT positionCaseInsensitive(1, '1')",
        "SELECT positionCaseInsensitive('a', 'a', 1, 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
