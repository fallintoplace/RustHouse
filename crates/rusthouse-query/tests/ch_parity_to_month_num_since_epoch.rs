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
        ("SELECT toMonthNumSinceEpoch('1970-01-01')", ConstantValue::Integer(0)),
        ("SELECT toMonthNumSinceEpoch('1970-02-01')", ConstantValue::Integer(1)),
        ("SELECT toMonthNumSinceEpoch('2000-01-01')", ConstantValue::Integer(360)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT toMonthNumSinceEpoch(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_date_inputs() {
    for sql in [
        "SELECT toMonthNumSinceEpoch()",
        "SELECT toMonthNumSinceEpoch('2023-02-29')",
        "SELECT toMonthNumSinceEpoch('bad-date')",
        "SELECT toMonthNumSinceEpoch(5)",
        
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
