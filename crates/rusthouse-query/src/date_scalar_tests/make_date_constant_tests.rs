use crate::{ConstantValue, Projection, parse_select};

#[test]
fn renders_valid_leap_year_and_null_dates() {
    for (sql, expected) in [
        (
            "SELECT makeDate(2024, 2, 29)",
            ConstantValue::Text("2024-02-29".into()),
        ),
        (
            "SELECT makeDate(1900, 1, 1)",
            ConstantValue::Text("1900-01-01".into()),
        ),
        ("SELECT makeDate(NULL, 1, 1)", ConstantValue::Null),
    ] {
        assert!(
            matches!(
                parse_select(sql).unwrap().projection,
                Projection::Constants(ref values)
                    if values.len() == 1 && values[0].value == expected
            ),
            "{sql}"
        );
    }
}

#[test]
fn refuses_invalid_calendar_fields_and_arity() {
    for sql in [
        "SELECT makeDate(2023, 2, 29)",
        "SELECT makeDate(2024, 13, 1)",
        "SELECT makeDate(2024, 1, 0)",
        "SELECT makeDate(1899, 1, 1)",
        "SELECT makeDate(2024, 1)",
        "SELECT makeDate(2024, 1, 1, 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
