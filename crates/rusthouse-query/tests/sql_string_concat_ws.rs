use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn concat_ws_skips_nulls_and_bounds_unicode_output() {
    for (sql, expected) in [
        ("SELECT CONCAT_WS('-', 'a', 'b', 'c')", "a-b-c"),
        ("SELECT concat_ws(', ', 'Ada', NULL, 'Grace')", "Ada, Grace"),
        ("SELECT CONCAT_WS('-', NULL, NULL)", ""),
        ("SELECT CONCAT_WS('-', '', 'b')", "-b"),
        ("SELECT CONCAT_WS('', 'é', '🙂')", "é🙂"),
        ("SELECT CONCAT_WS('-', 'a')", "a"),
        ("SELECT CONCAT_WS('-', UPPER('a'), 'b')", "A-b"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT CONCAT_WS(NULL, 'a', 'b')"),
        ConstantValue::Null
    );
}

#[test]
fn concat_ws_rejects_invalid_calls_and_oversized_results() {
    for sql in [
        "SELECT CONCAT_WS()",
        "SELECT CONCAT_WS('-')",
        "SELECT CONCAT_WS('-', 'a', 1)",
        "SELECT CONCAT_WS('-', 'a',)",
        "SELECT CONCAT_WS('', REPEAT('a', 1048576), 'x')",
        "SELECT CONCAT_WS('-', 'a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
