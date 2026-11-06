use crate::{ConstantValue, Projection, parse_select};

#[test]
fn returns_gregorian_month_lengths_and_preserves_null() {
    for (sql, expected) in [
        (
            "SELECT toDaysInMonth('2023-02-01')",
            ConstantValue::Integer(28),
        ),
        (
            "SELECT toDaysInMonth('2024-02-29')",
            ConstantValue::Integer(29),
        ),
        (
            "SELECT toDaysInMonth('2024-04-15')",
            ConstantValue::Integer(30),
        ),
        (
            "SELECT toDaysInMonth('2024-12-31')",
            ConstantValue::Integer(31),
        ),
        ("SELECT toDaysInMonth(NULL)", ConstantValue::Null),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection,
            Projection::Constants(ref values) if values.len() == 1
                && values[0].value == expected),
            "{sql}"
        );
    }
    assert!(parse_select("SELECT toDaysInMonth('2024-01-01', 1)").is_err());
}
