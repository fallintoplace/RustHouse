use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn evaluates_utc_parity_and_null_inputs() {
    assert_eq!(
        value("SELECT timeSlot('2024-02-29 23:59:59')"),
        ConstantValue::Text("2024-02-29 23:30:00".into())
    );
    assert_eq!(
        value("SELECT timeSlot('1970-01-01 00:00:00')"),
        ConstantValue::Text("1970-01-01 00:00:00".into())
    );
    assert_eq!(
        value("SELECT timeSlot(NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_unsupported_types_ranges_and_arity() {
    for sql in [
        "SELECT timeSlot()",
        "SELECT timeSlot(17)",
        "SELECT timeSlot('2023-02-29 23:59:59')",
        "SELECT timeSlot('2024-02-29 23:59:59', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
