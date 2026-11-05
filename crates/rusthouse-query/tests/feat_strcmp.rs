use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn strcmp_handles_constant_inputs() {
    for (sql, expected) in [
        ("SELECT strcmp('a','b')", ConstantValue::Integer(-1)),
        ("SELECT strcmp('a','a')", ConstantValue::Integer(0)),
        ("SELECT strcmp('b','a')", ConstantValue::Integer(1)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT strcmp(NULL, NULL)"), ConstantValue::Null);
}

#[test]
fn strcmp_rejects_malformed_inputs() {
    for sql in [
        "SELECT strcmp('x')",
        "SELECT strcmp('x','y','z')",
        "SELECT strcmp(1,'a')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
