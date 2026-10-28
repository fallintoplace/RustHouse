use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn matches_bounded_clickhouse_calendar_behavior() {
    for (sql, expected) in [
        ("SELECT toLastDayOfWeek('2023-04-21')", ConstantValue::Text('2023-04-22'.into())),
        ("SELECT toLastDayOfWeek('2023-04-21', 1)", ConstantValue::Text('2023-04-23'.into())),
        ("SELECT toLastDayOfWeek('2023-04-23', 1)", ConstantValue::Text('2023-04-23'.into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT toLastDayOfWeek(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_date_inputs() {
    for sql in [
        "SELECT toLastDayOfWeek()",
        "SELECT toLastDayOfWeek('2023-02-29')",
        "SELECT toLastDayOfWeek('bad-date')",
        "SELECT toLastDayOfWeek(5)",
        "SELECT toLastDayOfWeek('2023-04-21', 9)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
