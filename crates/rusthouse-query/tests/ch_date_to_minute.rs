use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_minute_handles_calendar_boundaries_and_null() {
    assert_eq!(
        value("SELECT toMinute('2024-02-29 23:45:09')"),
        ConstantValue::Integer(45)
    );
    assert_eq!(value("SELECT toMinute(NULL)"), ConstantValue::Null);
}

#[test]
fn to_minute_rejects_bad_dates_and_argument_shapes() {
    for sql in [
        "SELECT toMinute('2024-02-30 23:45:09')",
        "SELECT toMinute(1)",
        "SELECT toMinute()",
        "SELECT toMinute('2024-02-29 23:45:09', '2024-02-29 23:45:09')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
