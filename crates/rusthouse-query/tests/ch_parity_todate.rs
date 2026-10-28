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
        value("SELECT toDate('2024-02-29 23:59:59')"),
        ConstantValue::Text("2024-02-29".into())
    );
    assert_eq!(
        value("SELECT toDate('2024-02-29')"),
        ConstantValue::Text("2024-02-29".into())
    );
    assert_eq!(
        value("SELECT toDate('2024-02-29T01:00:00+02:00')"),
        ConstantValue::Text("2024-02-28".into())
    );
    assert_eq!(
        value("SELECT toDate(NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_bad_date_types_and_ranges() {
    for sql in [
        "SELECT toDate()",
        "SELECT toDate(42)",
        "SELECT toDate('2023-02-29')",
        "SELECT toDate('2024-02-29', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
