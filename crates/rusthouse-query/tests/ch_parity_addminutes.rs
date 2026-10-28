use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_utc_calendar_boundaries_and_null() {
    assert_eq!(
        value("SELECT addMinutes('2024-02-29 23:59:59', 1)"),
        ConstantValue::Text("2024-03-01 00:00:59".into())
    );
    assert_eq!(
        value("SELECT addMinutes('2024-02-29 00:01:00', -1)"),
        ConstantValue::Text("2024-02-29 00:00:00".into())
    );
    assert_eq!(
        value("SELECT addMinutes(NULL, 1)"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT addMinutes('2024-02-29 00:00:00', NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_bad_date_types_and_ranges() {
    for sql in [
        "SELECT addMinutes()",
        "SELECT addMinutes('2024-01-01')",
        "SELECT addMinutes('2024-02-29 00:00:00', 'x')",
        "SELECT addMinutes('1970-01-01 00:00:00', -1)",
        "SELECT addMinutes('2024-02-29 00:00:00', 9223372036854775807)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
