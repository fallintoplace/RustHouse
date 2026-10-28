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
        value("SELECT dateDiff('day', '2024-02-29 23:59:59', '2024-03-01 00:00:00')"),
        ConstantValue::Integer(1)
    );
    assert_eq!(
        value("SELECT dateDiff('minute', '2024-02-29 23:59:59', '2024-03-01 00:00:00')"),
        ConstantValue::Integer(1)
    );
    assert_eq!(
        value("SELECT dateDiff('day', '2024-03-01', '2024-02-29')"),
        ConstantValue::Integer(-1)
    );
    assert_eq!(
        value("SELECT dateDiff('second', '1970-01-01 00:00:00', '1970-01-01 00:00:59')"),
        ConstantValue::Integer(59)
    );
    assert_eq!(
        value("SELECT dateDiff(NULL, NULL, NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_unsupported_values_and_arity() {
    for sql in [
        "SELECT dateDiff()",
        "SELECT dateDiff('day', '2024-01-01')",
        "SELECT dateDiff('month', '2024-01-01', '2024-02-01')",
        "SELECT dateDiff('day', 'not a date', '2024-01-01')",
        "SELECT dateDiff('day', '2024-01-01', '2024-02-01', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
