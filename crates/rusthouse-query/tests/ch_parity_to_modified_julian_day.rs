use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn converts_modified_julian_days_in_bounded_calendar() {
    for (sql, expected) in [
        ("SELECT toModifiedJulianDay('1970-01-01')", ConstantValue::Integer(40587)),
        ("SELECT toModifiedJulianDay('2020-01-01')", ConstantValue::Integer(58849)),
        ("SELECT toModifiedJulianDay('2000-02-29')", ConstantValue::Integer(51603)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT toModifiedJulianDay(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_or_out_of_range_arguments() {
    for sql in [
        "SELECT toModifiedJulianDay()",
        "SELECT toModifiedJulianDay('2023-02-29')",
        "SELECT toModifiedJulianDay('2020-01-01 00:00:00')",
        "SELECT toModifiedJulianDay(58849)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
