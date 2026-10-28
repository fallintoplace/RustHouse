use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_bounded_calendar_and_null_values() {
    assert_eq!(
        value("SELECT addMonths('2024-01-31', 1)"),
        ConstantValue::Text("2024-02-29".into())
    );
    assert_eq!(
        value("SELECT addMonths('2023-01-31', 1)"),
        ConstantValue::Text("2023-02-28".into())
    );
    assert_eq!(
        value("SELECT addMonths('2024-02-29 12:34:56', 12)"),
        ConstantValue::Text("2025-02-28 12:34:56".into())
    );
    assert_eq!(
        value("SELECT addMonths('2024-03-31', -1)"),
        ConstantValue::Text("2024-02-29".into())
    );
    assert_eq!(
        value("SELECT addMonths(NULL, 1)"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT addMonths('2024-01-01', NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_bad_calendar_types_and_ranges() {
    for sql in [
        "SELECT addMonths()",
        "SELECT addMonths('bad', 1)",
        "SELECT addMonths('2149-06-06', 1)",
        "SELECT addMonths('2024-01-01', 1, 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
