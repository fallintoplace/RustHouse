use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn find_in_set_handles_constants() {
    for (sql, expected) in [
        ("SELECT find_in_set('b','a,b,c')", ConstantValue::Integer(2)),
        ("SELECT find_in_set('x','a,b,c')", ConstantValue::Integer(0)),
        (
            "SELECT find_in_set('a,b','a,b,c')",
            ConstantValue::Integer(0),
        ),
        ("SELECT find_in_set('é','a,é,b')", ConstantValue::Integer(2)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT find_in_set(NULL, NULL)"), ConstantValue::Null);
}
#[test]
fn find_in_set_rejects_invalid_arguments() {
    for sql in [
        "SELECT find_in_set('a')",
        "SELECT find_in_set('a','a','b')",
        "SELECT find_in_set(1,'a')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
