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
        ("SELECT toStartOfISOYear('2021-01-01')", ConstantValue::Text("2019-12-30".into())),
        ("SELECT toStartOfISOYear('2021-01-04')", ConstantValue::Text("2021-01-04".into())),
        ("SELECT toStartOfISOYear('2020-12-31')", ConstantValue::Text("2019-12-30".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT toStartOfISOYear(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_date_inputs() {
    for sql in [
        "SELECT toStartOfISOYear()",
        "SELECT toStartOfISOYear('2023-02-29')",
        "SELECT toStartOfISOYear('bad-date')",
        "SELECT toStartOfISOYear(5)",
        
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
