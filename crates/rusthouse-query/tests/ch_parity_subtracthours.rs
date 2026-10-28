use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn checks_utc_boundaries_and_null_values() {
    assert_eq!(
        value("SELECT subtractHours('2024-03-01 00:00:00', 1)"),
        ConstantValue::Text("2024-02-29 23:00:00".into())
    );
    assert_eq!(
        value("SELECT subtractHours('2024-03-01 00:00:00', -1)"),
        ConstantValue::Text("2024-03-01 01:00:00".into())
    );
    assert_eq!(
        value("SELECT subtractHours(NULL, 1)"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT subtractHours('2024-03-01 00:00:00', NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_unsupported_values_and_arity() {
    for sql in [
        "SELECT subtractHours()",
        "SELECT subtractHours('2024-03-01 00:00:00')",
        "SELECT subtractHours('2024-03-01 00:00:00', 'invalid')",
        "SELECT subtractHours('1970-01-01 00:00:00', 1)",
        "SELECT subtractHours('2024-03-01 00:00:00', 9223372036854775807)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
