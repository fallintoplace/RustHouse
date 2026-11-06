use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn from_base_handles_constant_inputs() {
    for (sql, expected) in [
        ("SELECT from_base('FF',16)", ConstantValue::Integer(255)),
        ("SELECT from_base('ff',16)", ConstantValue::Integer(255)),
        ("SELECT from_base('-1111',2)", ConstantValue::Integer(-15)),
        ("SELECT from_base('0',36)", ConstantValue::Integer(0)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT from_base(NULL, NULL)"), ConstantValue::Null);
}

#[test]
fn from_base_rejects_invalid_arguments() {
    for sql in [
        "SELECT from_base('g',16)",
        "SELECT from_base('9223372036854775808',10)",
        "SELECT from_base('',2)",
        "SELECT from_base('1',1)",
        "SELECT from_base('1')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
