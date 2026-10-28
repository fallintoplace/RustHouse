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
        value("SELECT toUnixTimestamp('2024-01-01 00:00:00')"),
        ConstantValue::Integer(1704067200)
    );
    assert_eq!(
        value("SELECT toUnixTimestamp('1970-01-01 00:00:00')"),
        ConstantValue::Integer(0)
    );
    assert_eq!(
        value("SELECT toUnixTimestamp('2024-01-01T01:00:00+01:00')"),
        ConstantValue::Integer(1704067200)
    );
    assert_eq!(
        value("SELECT toUnixTimestamp(NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_unsupported_types_ranges_and_arity() {
    for sql in [
        "SELECT toUnixTimestamp()",
        "SELECT toUnixTimestamp(17)",
        "SELECT toUnixTimestamp('2023-02-29 00:00:00')",
        "SELECT toUnixTimestamp('2024-01-01 00:00:00', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
