use crate::{ConstantValue, Projection, parse_select};

#[test]
fn monday_and_sunday_modes_match_calendar_weekdays() {
    for (sql, expected) in [
        ("SELECT toDayOfWeek('2023-04-21')", 5),
        ("SELECT toDayOfWeek('2023-04-21', 0)", 5),
        ("SELECT toDayOfWeek('2023-04-21', 1)", 4),
        ("SELECT toDayOfWeek('2023-04-21', 2)", 5),
        ("SELECT toDayOfWeek('2023-04-21', 3)", 6),
        ("SELECT toDayOfWeek('2023-04-23', 2)", 0),
        ("SELECT toDayOfWeek('2023-04-23', 3)", 1),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection,
            Projection::Constants(ref values) if values.len() == 1
                && values[0].value == ConstantValue::Integer(expected)),
            "{sql}"
        );
    }
}

#[test]
fn invalid_modes_and_extra_arguments_fail() {
    for sql in [
        "SELECT toDayOfWeek('2024-01-01', -1)",
        "SELECT toDayOfWeek('2024-01-01', 4)",
        "SELECT toDayOfWeek('2024-01-01', 0, 'UTC')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
