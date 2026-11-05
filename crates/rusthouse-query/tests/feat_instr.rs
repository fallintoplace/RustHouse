use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn instr_handles_constants() {
    for (sql, expected) in [
        ("SELECT instr('banana','na')", ConstantValue::Integer(3)),
        ("SELECT instr('banana','z')", ConstantValue::Integer(0)),
        ("SELECT instr('ééa','a')", ConstantValue::Integer(3)),
        ("SELECT instr('abc','')", ConstantValue::Integer(1)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT instr(NULL, NULL)"), ConstantValue::Null);
}
#[test]
fn instr_rejects_invalid_arguments() {
    for sql in [
        "SELECT instr('a')",
        "SELECT instr('a','b','c')",
        "SELECT instr(1,'a')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
