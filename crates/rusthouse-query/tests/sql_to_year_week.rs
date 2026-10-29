use rusthouse_query::{parse_select, ConstantValue, Projection};

#[test]
fn year_week_mode_zero_crosses_calendar_years() {
    for (date, expected) in [
        ("2021-01-01", 202052),
        ("2021-01-03", 202101),
        ("2017-01-01", 201701),
        ("2016-12-31", 201652),
    ] {
        let sql = format!("SELECT toYearWeek('{date}')");
        let result = parse_select(&sql).unwrap();
        assert!(matches!(result.projection, Projection::Constants(values)
            if values[0].value == ConstantValue::Integer(expected)), "{sql}");
    }
    assert!(parse_select("SELECT toYearWeek('2024-02-30')").is_err());
    assert!(parse_select("SELECT toYearWeek('2024-01-01', 3)").is_err());
}
