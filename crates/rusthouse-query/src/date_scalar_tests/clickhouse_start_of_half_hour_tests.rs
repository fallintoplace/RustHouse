use crate::{ConstantValue, Projection, parse_select};

#[test]
fn rounds_utc_timestamps_down_to_half_hour() {
    for (sql, expected) in [
        (
            "SELECT toStartOfHalfHour('2024-01-01 12:29:59')",
            "2024-01-01 12:00:00",
        ),
        (
            "SELECT toStartOfHalfHour('2024-01-01 12:30:00')",
            "2024-01-01 12:30:00",
        ),
        (
            "SELECT toStartOfHalfHour('2024-01-01 23:59:59')",
            "2024-01-01 23:30:00",
        ),
        (
            "SELECT toStartOfHalfHour('2024-02-29')",
            "2024-02-29 00:00:00",
        ),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Text(expected.into())),
            "{sql}",
        );
    }
    assert!(
        matches!(parse_select("SELECT toStartOfHalfHour(NULL)").unwrap().projection,
            Projection::Constants(values) if values[0].value == ConstantValue::Null)
    );
    assert!(parse_select("SELECT toStartOfHalfHour('2024-01-01', 1)").is_err());
}
