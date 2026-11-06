use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extract_handles_constant_inputs() {
    for (sql, expected) in [
        (
            "SELECT extract('abc-123','([0-9]+)')",
            ConstantValue::Text("123".into()),
        ),
        (
            "SELECT extract('abc123','[0-9]+')",
            ConstantValue::Text("123".into()),
        ),
        (
            "SELECT extract('abc','[0-9]+')",
            ConstantValue::Text("".into()),
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT extract(NULL, NULL)"), ConstantValue::Null);
}

#[test]
fn extract_rejects_malformed_inputs() {
    for sql in [
        "SELECT extract('x', '[')",
        "SELECT extract('x')",
        "SELECT extract('x', 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
