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
        value("SELECT toDateTime('2024-02-29')"),
        ConstantValue::Text("2024-02-29 00:00:00".into())
    );
    assert_eq!(
        value("SELECT toDateTime('2024-02-29 23:59:59')"),
        ConstantValue::Text("2024-02-29 23:59:59".into())
    );
    assert_eq!(
        value("SELECT toDateTime(NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_bad_date_types_and_ranges() {
    for sql in [
        "SELECT toDateTime()",
        "SELECT toDateTime(42)",
        "SELECT toDateTime('2023-02-29')",
        "SELECT toDateTime('2149-06-06')",
        "SELECT toDateTime('2024-02-29', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
