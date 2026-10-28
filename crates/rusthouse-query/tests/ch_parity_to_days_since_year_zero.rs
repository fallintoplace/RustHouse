use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn converts_bounded_utc_date_constants() {
    for (sql, expected) in [
        ("SELECT toDaysSinceYearZero('1970-01-01')", ConstantValue::Integer(719528)),
        ("SELECT toDaysSinceYearZero('2000-02-29')", ConstantValue::Integer(730544)),
        ("SELECT toDaysSinceYearZero('2023-09-08')", ConstantValue::Integer(739136)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT toDaysSinceYearZero(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_or_out_of_range_inputs() {
    for sql in [
        "SELECT toDaysSinceYearZero()",
        "SELECT toDaysSinceYearZero('2023-02-29')",
        "SELECT toDaysSinceYearZero('nonsense')",
        "SELECT toDaysSinceYearZero(5)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
