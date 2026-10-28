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
        ("SELECT fromModifiedJulianDay(40587)", ConstantValue::Text("1970-01-01".into())),
        ("SELECT fromModifiedJulianDay(58849)", ConstantValue::Text("2020-01-01".into())),
        ("SELECT fromModifiedJulianDay(51603)", ConstantValue::Text("2000-02-29".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT fromModifiedJulianDay(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_or_out_of_range_arguments() {
    for sql in [
        "SELECT fromModifiedJulianDay()",
        "SELECT fromModifiedJulianDay(40586)",
        "SELECT fromModifiedJulianDay(106123)",
        "SELECT fromModifiedJulianDay('2020-01-01')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
