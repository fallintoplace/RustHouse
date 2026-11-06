use super::*;

fn value(sql: &str) -> ConstantValue {
    match crate::parse_select(sql).expect(sql).projection {
        crate::Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        crate::Projection::Literal(v) => ConstantValue::Integer(v),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn extracts_minute_second_and_converts_utc_date_constants() {
    for (sql, expected) in [
        (
            "SELECT toMinute('2024-02-29 23:59:08')",
            ConstantValue::Integer(59),
        ),
        (
            "SELECT toSecond('2024-02-29 23:59:08')",
            ConstantValue::Integer(8),
        ),
        (
            "SELECT toDate('2024-02-29 23:59:08')",
            ConstantValue::Text("2024-02-29".into()),
        ),
        (
            "SELECT toDateTime('2024-02-29')",
            ConstantValue::Text("2024-02-29 00:00:00".into()),
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    for function in ["toMinute", "toSecond", "toDate", "toDateTime"] {
        assert_eq!(
            value(&format!("SELECT {function}(NULL)")),
            ConstantValue::Null
        );
        assert!(crate::parse_select(&format!("SELECT {function}('not-a-date')")).is_err());
        assert!(crate::parse_select(&format!("SELECT {function}('2024-01-01', 1)")).is_err());
    }
}

#[test]
fn parses_ascii_temporal_digits_at_calendar_boundaries() {
    let value = Temporal::parse("2024-02-29 23:59:58").unwrap();
    assert_eq!(
        Temporal::render(value.seconds, true).unwrap(),
        "2024-02-29 23:59:58"
    );
    assert!(Temporal::parse("2024-02-30").is_err());
    assert!(Temporal::parse("2024-02-29 24:00:00").is_err());
}

#[test]
fn subtract_days_handles_leap_dates_and_bounds() {
    for (sql, expected) in [
        ("SELECT subtractDays('2024-03-01', 1)", "2024-02-29"),
        ("SELECT subtractDays('2024-01-01', -1)", "2024-01-02"),
        (
            "SELECT subtractDays('2024-03-01 01:30:00', 1)",
            "2024-02-29 01:30:00",
        ),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT subtractDays(NULL, 1)"), ConstantValue::Null);
    for sql in [
        "SELECT subtractDays()",
        "SELECT subtractDays('2023-02-29', 1)",
        "SELECT subtractDays('1900-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn add_weeks_handles_utc_and_bounds() {
    for (sql, expected) in [
        ("SELECT addWeeks('2024-02-22', 1)", "2024-02-29"),
        ("SELECT addWeeks('2024-02-29', -1)", "2024-02-22"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addWeeks(NULL, 1)"), ConstantValue::Null);
    for sql in [
        "SELECT addWeeks()",
        "SELECT addWeeks('2023-02-29', 1)",
        "SELECT addWeeks('2299-12-31', 1)",
        "SELECT addWeeks('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn subtract_weeks_handles_utc_and_bounds() {
    for (sql, expected) in [
        ("SELECT subtractWeeks('2024-03-07', 1)", "2024-02-29"),
        ("SELECT subtractWeeks('2024-02-22', -1)", "2024-02-29"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT subtractWeeks(NULL, 1)"), ConstantValue::Null);
    for sql in [
        "SELECT subtractWeeks()",
        "SELECT subtractWeeks('2023-02-29', 1)",
        "SELECT subtractWeeks('1900-01-01', 1)",
        "SELECT subtractWeeks('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn add_hours_handles_utc_and_bounds() {
    for (sql, expected) in [
        (
            "SELECT addHours('2024-02-29 23:30:00', 1)",
            "2024-03-01 00:30:00",
        ),
        ("SELECT addHours('2024-02-29', 1)", "2024-02-29 01:00:00"),
        (
            "SELECT addHours('2024-02-29 23:30:00', -1)",
            "2024-02-29 22:30:00",
        ),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addHours(NULL, 1)"), ConstantValue::Null);
    for sql in [
        "SELECT addHours()",
        "SELECT addHours('2023-02-29', 1)",
        "SELECT addHours('2299-12-31 23:59:59', 1)",
        "SELECT addHours('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn add_minutes_handles_utc_and_bounds() {
    for (sql, expected) in [
        (
            "SELECT addMinutes('2024-02-29 23:59:30', 1)",
            "2024-03-01 00:00:30",
        ),
        ("SELECT addMinutes('2024-02-29', 1)", "2024-02-29 00:01:00"),
        (
            "SELECT addMinutes('2024-02-29 23:59:30', -1)",
            "2024-02-29 23:58:30",
        ),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addMinutes(NULL, 1)"), ConstantValue::Null);
    for sql in [
        "SELECT addMinutes()",
        "SELECT addMinutes('2023-02-29', 1)",
        "SELECT addMinutes('2299-12-31 23:59:59', 1)",
        "SELECT addMinutes('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn add_seconds_handles_utc_and_bounds() {
    for (sql, expected) in [
        (
            "SELECT addSeconds('2024-02-29 23:59:59', 1)",
            "2024-03-01 00:00:00",
        ),
        ("SELECT addSeconds('2024-02-29', 1)", "2024-02-29 00:00:01"),
        (
            "SELECT addSeconds('2024-02-29 23:59:59', -1)",
            "2024-02-29 23:59:58",
        ),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addSeconds(NULL, 1)"), ConstantValue::Null);
    for sql in [
        "SELECT addSeconds()",
        "SELECT addSeconds('2023-02-29', 1)",
        "SELECT addSeconds('2299-12-31 23:59:59', 1)",
        "SELECT addSeconds('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn subtract_seconds_handles_utc_and_bounds() {
    for (sql, expected) in [
        (
            "SELECT subtractSeconds('2024-03-01 00:00:00', 1)",
            "2024-02-29 23:59:59",
        ),
        (
            "SELECT subtractSeconds('2024-02-29', 1)",
            "2024-02-28 23:59:59",
        ),
        (
            "SELECT subtractSeconds('2024-03-01 00:00:00', -1)",
            "2024-03-01 00:00:01",
        ),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT subtractSeconds(NULL, 1)"),
        ConstantValue::Null
    );
    for sql in [
        "SELECT subtractSeconds()",
        "SELECT subtractSeconds('2023-02-29', 1)",
        "SELECT subtractSeconds('1900-01-01 00:00:00', 1)",
        "SELECT subtractSeconds('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn add_years_clamps_month_ends_safely() {
    for (sql, expected) in [
        ("SELECT addYears('2024-02-29', 1)", "2025-02-28"),
        (
            "SELECT addYears('2024-01-31 12:30:05', 1)",
            "2025-01-31 12:30:05",
        ),
        ("SELECT addYears('2023-02-28', 1)", "2024-02-28"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addYears(NULL, 1)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT addYears('2024-02-29', NULL)"),
        ConstantValue::Null
    );
    for sql in [
        "SELECT addYears()",
        "SELECT addYears('2023-02-29', 1)",
        "SELECT addYears('2299-12-31', 1)",
        "SELECT addYears('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn subtract_years_clamps_month_ends_safely() {
    for (sql, expected) in [
        ("SELECT subtractYears('2024-02-29', 1)", "2023-02-28"),
        ("SELECT subtractYears('2024-02-29', -4)", "2028-02-29"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT subtractYears(NULL, 1)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT subtractYears('2024-02-29', NULL)"),
        ConstantValue::Null
    );
    for sql in [
        "SELECT subtractYears()",
        "SELECT subtractYears('2023-02-29', 1)",
        "SELECT subtractYears('1900-01-01', 1)",
        "SELECT subtractYears('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn add_quarters_clamps_month_ends_safely() {
    for (sql, expected) in [
        ("SELECT addQuarters('2024-01-31', 1)", "2024-04-30"),
        ("SELECT addQuarters('2024-11-30', 1)", "2025-02-28"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addQuarters(NULL, 1)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT addQuarters('2024-02-29', NULL)"),
        ConstantValue::Null
    );
    for sql in [
        "SELECT addQuarters()",
        "SELECT addQuarters('2023-02-29', 1)",
        "SELECT addQuarters('2299-11-30', 1)",
        "SELECT addQuarters('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn subtract_quarters_clamps_month_ends_safely() {
    for (sql, expected) in [
        ("SELECT subtractQuarters('2024-05-31', 1)", "2024-02-29"),
        ("SELECT subtractQuarters('2024-02-29', -1)", "2024-05-29"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT subtractQuarters(NULL, 1)"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT subtractQuarters('2024-02-29', NULL)"),
        ConstantValue::Null
    );
    for sql in [
        "SELECT subtractQuarters()",
        "SELECT subtractQuarters('2023-02-29', 1)",
        "SELECT subtractQuarters('1900-02-01', 1)",
        "SELECT subtractQuarters('2024-01-01', 1, 2)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn start_day_preserves_calendar_boundaries() {
    for (sql, expected) in [
        (
            "SELECT toStartOfDay('2024-02-29 12:34:56')",
            "2024-02-29 00:00:00",
        ),
        ("SELECT toStartOfDay('2024-02-29')", "2024-02-29 00:00:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfDay(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toStartOfDay()",
        "SELECT toStartOfDay('2023-02-29')",
        "SELECT toStartOfDay('2024-03-06', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn start_hour_preserves_calendar_boundaries() {
    for (sql, expected) in [
        (
            "SELECT toStartOfHour('2024-02-29 12:34:56')",
            "2024-02-29 12:00:00",
        ),
        ("SELECT toStartOfHour('2024-02-29')", "2024-02-29 00:00:00"),
        (
            "SELECT toStartOfHour('2024-03-01 00:00:59')",
            "2024-03-01 00:00:00",
        ),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfHour(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toStartOfHour()",
        "SELECT toStartOfHour('2023-02-29')",
        "SELECT toStartOfHour('2024-03-06', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn start_week_preserves_calendar_boundaries() {
    for (sql, expected) in [
        ("SELECT toStartOfWeek('2024-03-06')", "2024-03-03"),
        ("SELECT toStartOfWeek('2024-03-03')", "2024-03-03"),
        ("SELECT toStartOfWeek('2024-03-06 12:30:00')", "2024-03-03"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfWeek(NULL)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT toStartOfWeek('2024-03-06', 1)"),
        ConstantValue::Text("2024-03-04".into())
    );
    for sql in [
        "SELECT toStartOfWeek()",
        "SELECT toStartOfWeek('2023-02-29')",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn to_monday_preserves_calendar_boundaries() {
    for (sql, expected) in [
        ("SELECT toMonday('2024-03-06')", "2024-03-04"),
        ("SELECT toMonday('2024-03-03')", "2024-02-26"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toMonday(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toMonday()",
        "SELECT toMonday('2023-02-29')",
        "SELECT toMonday('2024-03-06', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn start_month_uses_gregorian_calendar() {
    for (sql, expected) in [
        ("SELECT toStartOfMonth('2024-02-29')", "2024-02-01"),
        ("SELECT toStartOfMonth('2024-12-31 12:34:56')", "2024-12-01"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfMonth(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toStartOfMonth()",
        "SELECT toStartOfMonth('2023-02-29')",
        "SELECT toStartOfMonth('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn start_quarter_uses_gregorian_calendar() {
    for (sql, expected) in [
        ("SELECT toStartOfQuarter('2024-05-31')", "2024-04-01"),
        ("SELECT toStartOfQuarter('2024-01-01')", "2024-01-01"),
        ("SELECT toStartOfQuarter('2024-12-31')", "2024-10-01"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfQuarter(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toStartOfQuarter()",
        "SELECT toStartOfQuarter('2023-02-29')",
        "SELECT toStartOfQuarter('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn start_year_uses_gregorian_calendar() {
    for (sql, expected) in [
        ("SELECT toStartOfYear('2024-02-29')", "2024-01-01"),
        ("SELECT toStartOfYear('2025-12-31 23:59:59')", "2025-01-01"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfYear(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toStartOfYear()",
        "SELECT toStartOfYear('2023-02-29')",
        "SELECT toStartOfYear('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn last_day_month_uses_gregorian_calendar() {
    for (sql, expected) in [
        ("SELECT toLastDayOfMonth('2024-02-01')", "2024-02-29"),
        ("SELECT toLastDayOfMonth('2023-02-15')", "2023-02-28"),
        ("SELECT toLastDayOfMonth('2024-12-31')", "2024-12-31"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toLastDayOfMonth(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toLastDayOfMonth()",
        "SELECT toLastDayOfMonth('2023-02-29')",
        "SELECT toLastDayOfMonth('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn day_of_month_extracts_correct_calendar_values() {
    for (sql, expected) in [
        ("SELECT toDayOfMonth('2024-02-29')", 29),
        ("SELECT toDayOfMonth('2024-01-01')", 1),
        ("SELECT toDayOfMonth('2024-12-31 23:59:59')", 31),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toDayOfMonth(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toDayOfMonth()",
        "SELECT toDayOfMonth('2023-02-29')",
        "SELECT toDayOfMonth('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn day_of_week_extracts_correct_calendar_values() {
    for (sql, expected) in [
        ("SELECT toDayOfWeek('2024-03-04')", 1),
        ("SELECT toDayOfWeek('2024-03-03')", 7),
        ("SELECT toDayOfWeek('1970-01-01')", 4),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toDayOfWeek(NULL)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT toDayOfWeek('2024-01-01', 1)"),
        ConstantValue::Integer(0)
    );
    for sql in ["SELECT toDayOfWeek()", "SELECT toDayOfWeek('2023-02-29')"] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn day_of_year_extracts_correct_calendar_values() {
    for (sql, expected) in [
        ("SELECT toDayOfYear('2024-12-31')", 366),
        ("SELECT toDayOfYear('2023-12-31')", 365),
        ("SELECT toDayOfYear('2024-01-01')", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toDayOfYear(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toDayOfYear()",
        "SELECT toDayOfYear('2023-02-29')",
        "SELECT toDayOfYear('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn to_month_extracts_correct_calendar_values() {
    for (sql, expected) in [
        ("SELECT toMonth('2024-02-29')", 2),
        ("SELECT toMonth('2024-12-01')", 12),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toMonth(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toMonth()",
        "SELECT toMonth('2023-02-29')",
        "SELECT toMonth('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn to_quarter_extracts_correct_calendar_values() {
    for (sql, expected) in [
        ("SELECT toQuarter('2024-03-31')", 1),
        ("SELECT toQuarter('2024-04-01')", 2),
        ("SELECT toQuarter('2024-12-31')", 4),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toQuarter(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toQuarter()",
        "SELECT toQuarter('2023-02-29')",
        "SELECT toQuarter('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn to_yyyymm_extracts_correct_calendar_values() {
    for (sql, expected) in [
        ("SELECT toYYYYMM('2024-02-29')", 202402),
        ("SELECT toYYYYMM('2024-12-31')", 202412),
        ("SELECT toYYYYMM('1900-01-01')", 190001),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toYYYYMM(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toYYYYMM()",
        "SELECT toYYYYMM('2023-02-29')",
        "SELECT toYYYYMM('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn to_yyyymmdd_matches_utc_calendar_examples() {
    for (sql, expected) in [
        ("SELECT toYYYYMMDD('2024-02-29')", 20240229),
        ("SELECT toYYYYMMDD('2299-12-31')", 22991231),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toYYYYMMDD(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toYYYYMMDD()",
        "SELECT toYYYYMMDD('2023-02-29')",
        "SELECT toYYYYMMDD('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn to_yyyymmddhhmmss_matches_utc_calendar_examples() {
    for (sql, expected) in [
        (
            "SELECT toYYYYMMDDhhmmss('2024-02-29 23:59:59')",
            20240229235959,
        ),
        ("SELECT toYYYYMMDDhhmmss('2024-02-29')", 20240229000000),
        (
            "SELECT toYYYYMMDDhhmmss('2299-12-31 23:59:59')",
            22991231235959,
        ),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toYYYYMMDDhhmmss(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toYYYYMMDDhhmmss()",
        "SELECT toYYYYMMDDhhmmss('2023-02-29')",
        "SELECT toYYYYMMDDhhmmss('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn iso_week_matches_utc_calendar_examples() {
    for (sql, expected) in [
        ("SELECT toISOWeek('2021-01-01')", 53),
        ("SELECT toISOWeek('2021-01-04')", 1),
        ("SELECT toISOWeek('2020-12-31')", 53),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toISOWeek(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toISOWeek()",
        "SELECT toISOWeek('2023-02-29')",
        "SELECT toISOWeek('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn iso_year_matches_utc_calendar_examples() {
    for (sql, expected) in [
        ("SELECT toISOYear('2021-01-01')", 2020),
        ("SELECT toISOYear('2021-01-04')", 2021),
        ("SELECT toISOYear('2019-12-30')", 2020),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toISOYear(NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT toISOYear()",
        "SELECT toISOYear('2023-02-29')",
        "SELECT toISOYear('2024-01-01', 1)",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn time_diff_matches_utc_calendar_examples() {
    for (sql, expected) in [
        (
            "SELECT timeDiff('2024-02-29 23:59:59', '2024-03-01 00:00:00')",
            1,
        ),
        (
            "SELECT timeDiff('2024-03-01 00:00:00', '2024-02-29 23:59:59')",
            -1,
        ),
        ("SELECT timeDiff('2024-02-28', '2024-03-01')", 172800),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT timeDiff(NULL, '2024-01-01')"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT timeDiff('2024-01-01', NULL)"),
        ConstantValue::Null
    );
    for sql in [
        "SELECT timeDiff()",
        "SELECT timeDiff('2023-02-29', '2024-01-01')",
        "SELECT timeDiff('2024-01-01')",
        "SELECT timeDiff('2024-01-01', '2024-01-02', '2024-01-03')",
    ] {
        assert!(crate::parse_select(sql).is_err(), "{sql}");
    }
}
