use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_count_lines() {
    for (sql, expected) in [
        ("SELECT countLines('a\\nb')", ConstantValue::Integer(2)),
        ("SELECT countLines('one')", ConstantValue::Integer(1)),
        ("SELECT countLines('')", ConstantValue::Integer(0)),
        ("SELECT countLines(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_count_lines_arity() {
    for sql in ["SELECT countLines()", "SELECT countLines('x', 'y')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
