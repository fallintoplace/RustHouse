use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn locate_handles_constants() {
    for (sql, expected) in [
        ("SELECT locate('a', 'banana')", ConstantValue::Integer(2)),
        ("SELECT locate('a', 'banana', 3)", ConstantValue::Integer(4)),
        ("SELECT locate('z','banana')", ConstantValue::Integer(0)),
        ("SELECT locate('é','aéé')", ConstantValue::Integer(2)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT locate(NULL, NULL)"), ConstantValue::Null);
}
#[test]
fn locate_rejects_invalid_arguments() {
    for sql in [
        "SELECT locate('a')",
        "SELECT locate('a','abc',1,2)",
        "SELECT locate(1,'abc')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
