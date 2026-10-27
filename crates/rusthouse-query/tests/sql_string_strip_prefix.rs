use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn strip_prefix_handles_constants_and_nulls() {
    for (sql, expected) in [
        ("SELECT STRIP_PREFIX('foobar', 'foo')", "bar"),
        ("SELECT strip_prefix('foobar','none')", "foobar"),
        ("SELECT STRIP_PREFIX('préfix','pré')", "fix"),
        ("SELECT STRIP_PREFIX('a','')", "a"),
        ("SELECT CONCAT(STRIP_PREFIX('abc','a'),'z')", "bcz"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT STRIP_PREFIX(NULL, 'a')"), ConstantValue::Null);
    assert_eq!(value("SELECT STRIP_PREFIX('a', NULL)"), ConstantValue::Null);
}

#[test]
fn strip_prefix_rejects_invalid_arguments() {
    for sql in [
        "SELECT STRIP_PREFIX()",
        "SELECT STRIP_PREFIX('a')",
        "SELECT STRIP_PREFIX('a','b','c')",
        "SELECT STRIP_PREFIX('a',1)",
        "SELECT STRIP_PREFIX('a','a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
