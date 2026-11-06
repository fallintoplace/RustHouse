use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn regexp_instr_supports_constant_patterns() {
    for (sql, expected) in [
        (
            "SELECT regexp_instr('abc123','[0-9]+')",
            ConstantValue::Integer(4),
        ),
        (
            "SELECT regexp_instr('abc','[0-9]+')",
            ConstantValue::Integer(0),
        ),
        (
            "SELECT regexp_instr('ééA3','[0-9]')",
            ConstantValue::Integer(4),
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT regexp_instr(NULL, 'x')"), ConstantValue::Null);
    assert_eq!(value("SELECT regexp_instr('x', NULL)"), ConstantValue::Null);
}

#[test]
fn regexp_instr_rejects_bad_patterns_and_arguments() {
    for sql in [
        "SELECT regexp_instr('x', '[')",
        "SELECT regexp_instr('x')",
        "SELECT regexp_instr('x', 'x', 'y')",
        "SELECT regexp_instr(1, 'x')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
