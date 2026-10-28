use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_start_of_hour_handles_utc_datetime_and_null() {
    assert_eq!(
        value("SELECT toStartOfHour('2024-02-29 23:45:09')"),
        ConstantValue::Text("2024-02-29 23:00:00".into())
    );
    assert_eq!(value("SELECT toStartOfHour(NULL)"), ConstantValue::Null);
}

#[test]
fn to_start_of_hour_rejects_invalid_datetime_and_arity() {
    for sql in [
        "SELECT toStartOfHour('2023-02-29 23:45:09')",
        "SELECT toStartOfHour(1)",
        "SELECT toStartOfHour()",
        "SELECT toStartOfHour('2024-02-29 23:45:09', '2024-02-29 23:45:09')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
