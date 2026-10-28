use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn recognizes_ascii_case_insensitive_subsequences() {
    for (sql, expected) in [
        ("SELECT hasSubsequenceCaseInsensitive('garbage', 'ARG')", 1),
        (
            "SELECT hasSubsequenceCaseInsensitive('Hello World', 'HLWRD')",
            1,
        ),
        ("SELECT hasSubsequenceCaseInsensitive('abc', 'CA')", 0),
        ("SELECT hasSubsequenceCaseInsensitive('abc', '')", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT hasSubsequenceCaseInsensitive(NULL, 'a')"),
        ConstantValue::Null
    );
}
#[test]
fn rejects_wrong_arity_and_types() {
    for sql in [
        "SELECT hasSubsequenceCaseInsensitive('a')",
        "SELECT hasSubsequenceCaseInsensitive(1, 'a')",
        "SELECT hasSubsequenceCaseInsensitive('a', 'b', 'c')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
