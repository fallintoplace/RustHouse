use crate::{ConstantValue, Projection, parse_select};
#[test]
fn formats_second_intervals_and_unit_limits() {
    for (sql, expected) in [
        ("SELECT formatReadableTimeDelta(0)", "0 seconds"),
        (
            "SELECT formatReadableTimeDelta(12345)",
            "3 hours, 25 minutes and 45 seconds",
        ),
        (
            "SELECT formatReadableTimeDelta(100, 'minutes')",
            "1 minute and 40 seconds",
        ),
        (
            "SELECT formatReadableTimeDelta(12345, 'minutes')",
            "205 minutes and 45 seconds",
        ),
        ("SELECT formatReadableTimeDelta(-60)", "-1 minute"),
        (
            "SELECT formatReadableTimeDelta(3600, 'minutes', 'minutes')",
            "60 minutes",
        ),
        (
            "SELECT formatReadableTimeDelta(5456456)",
            "2 months, 2 days, 3 hours, 40 minutes and 56 seconds",
        ),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
            if values.len() == 1 && values[0].value == ConstantValue::Text(expected.into())),
            "{sql}"
        );
    }
    assert!(parse_select("SELECT formatReadableTimeDelta(60, 'invalid')").is_err());
    assert!(parse_select("SELECT formatReadableTimeDelta(60, 'minutes', 'hours')").is_err());
}
