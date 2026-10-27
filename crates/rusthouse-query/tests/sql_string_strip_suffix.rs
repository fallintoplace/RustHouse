use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn strip_suffix_handles_constants_and_nulls() {
    for (sql, expected) in [
        ("SELECT STRIP_SUFFIX('hello','lo')", "hel"),
        ("SELECT strip_suffix('hello','bye')", "hello"),
        ("SELECT STRIP_SUFFIX('漢字','字')", "漢"),
        ("SELECT STRIP_SUFFIX('a','')", "a"),
        ("SELECT CONCAT(STRIP_SUFFIX('abc','c'),'z')", "abz"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT STRIP_SUFFIX(NULL, 'a')"), ConstantValue::Null);
    assert_eq!(value("SELECT STRIP_SUFFIX('a', NULL)"), ConstantValue::Null);
}

#[test]
fn strip_suffix_rejects_invalid_arguments() {
    for sql in [
        "SELECT STRIP_SUFFIX()",
        "SELECT STRIP_SUFFIX('a')",
        "SELECT STRIP_SUFFIX('a','b','c')",
        "SELECT STRIP_SUFFIX('a',1)",
        "SELECT STRIP_SUFFIX('a','a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
