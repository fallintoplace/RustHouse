use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn finds_ordered_noncontiguous_subsequences() {
    for (sql, expected) in [
        ("SELECT hasSubsequence('Hello World', 'HlWrd')", 1),
        ("SELECT hasSubsequence('abc', 'ac')", 1),
        ("SELECT hasSubsequence('abc', 'ca')", 0),
        ("SELECT hasSubsequence('abc', '')", 1),
        ("SELECT hasSubsequence('', 'x')", 0),
        ("SELECT hasSubsequence('ABC', 'a')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT hasSubsequence(NULL, 'a')"),
        ConstantValue::Null
    );
}
#[test]
fn requires_two_string_arguments() {
    for sql in [
        "SELECT hasSubsequence('a')",
        "SELECT hasSubsequence(1, 'a')",
        "SELECT hasSubsequence('a', 1)",
        "SELECT hasSubsequence('a', 'b', 'c')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
