use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn last_index_of_counts_unicode_positions() {
    for (sql, expected) in [
        ("SELECT LAST_INDEX_OF('banana', 'ana')", 4),
        ("SELECT last_index_of('é🙂a🙂', '🙂')", 4),
        ("SELECT LAST_INDEX_OF('abc', 'z')", 0),
        ("SELECT LAST_INDEX_OF('abc', '')", 4),
        ("SELECT LAST_INDEX_OF('', '')", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT LAST_INDEX_OF(NULL, 'a')"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT LAST_INDEX_OF('a', NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn last_index_of_rejects_bad_arguments() {
    for sql in [
        "SELECT LAST_INDEX_OF()",
        "SELECT LAST_INDEX_OF('a')",
        "SELECT LAST_INDEX_OF('a', 'b', 'c')",
        "SELECT LAST_INDEX_OF(1, 'a')",
        "SELECT LAST_INDEX_OF('a', 'a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
