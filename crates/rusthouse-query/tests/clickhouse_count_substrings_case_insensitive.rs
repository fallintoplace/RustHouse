use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn counts_non_overlapping_substrings_with_optional_start() {
    for (sql, expected) in [
        ("SELECT countSubstringsCaseInsensitive('AAAA', 'aa')", 2),
        ("SELECT countSubstringsCaseInsensitive('abc___ABC___abc', 'abc', 4)", 2),
        ("SELECT countSubstringsCaseInsensitive('aaA', 'aA')", 1),
        ("SELECT countSubstringsCaseInsensitive('hi', '')", 0),
        ("SELECT countSubstringsCaseInsensitive('abc', 'x')", 0),
    ] { assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}"); }
    assert_eq!(value("SELECT countSubstringsCaseInsensitive(NULL, 'a')"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_count_arguments() {
    for sql in [
        "SELECT countSubstringsCaseInsensitive('a')",
        "SELECT countSubstringsCaseInsensitive('a', 'a', -1)",
        "SELECT countSubstringsCaseInsensitive(1, 'a')",
        "SELECT countSubstringsCaseInsensitive('a', 'a', 1, 2)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
