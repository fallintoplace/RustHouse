use crate::{ConstantValue, Projection, parse_select};

#[test]
fn renders_utc_datetime_components_and_null() {
    for (sql, expected) in [
        (
            "SELECT makeDateTime(2024, 2, 29, 23, 59, 58)",
            ConstantValue::Text("2024-02-29 23:59:58".into()),
        ),
        (
            "SELECT makeDateTime(1900, 1, 1, 0, 0, 0)",
            ConstantValue::Text("1900-01-01 00:00:00".into()),
        ),
        (
            "SELECT makeDateTime(2024, NULL, 1, 0, 0, 0)",
            ConstantValue::Null,
        ),
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
fn rejects_invalid_time_date_and_arity() {
    for sql in [
        "SELECT makeDateTime(2023, 2, 29, 0, 0, 0)",
        "SELECT makeDateTime(2024, 2, 29, 24, 0, 0)",
        "SELECT makeDateTime(2024, 2, 29, 23, 60, 0)",
        "SELECT makeDateTime(2024, 2, 29, 23, 59, 60)",
        "SELECT makeDateTime(2024, 2, 29, 1, 2)",
        "SELECT makeDateTime(2024, 2, 29, 1, 2, 3, 4)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
