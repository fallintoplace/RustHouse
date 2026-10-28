use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_hour_handles_calendar_boundaries_and_null() {
    assert_eq!(
        value("SELECT toHour('2024-02-29 23:45:09')"),
        ConstantValue::Integer(23)
    );
    assert_eq!(value("SELECT toHour(NULL)"), ConstantValue::Null);
}

#[test]
fn to_hour_rejects_bad_dates_and_argument_shapes() {
    for sql in [
        "SELECT toHour('2024-02-30 23:45:09')",
        "SELECT toHour(1)",
        "SELECT toHour()",
        "SELECT toHour('2024-02-29 23:45:09', '2024-02-29 23:45:09')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
