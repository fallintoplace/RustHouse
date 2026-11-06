use crate::{ConstantValue, Projection, parse_select};

#[test]
fn false_and_null_conditions_yield_zero() {
    for sql in [
        "SELECT throwIf(0)",
        "SELECT throwIf(NULL)",
        "SELECT throwIf(0, 'unused')",
    ] {
        assert!(
            matches!(
                parse_select(sql).unwrap().projection,
                Projection::Constants(ref values)
                    if values.len() == 1 && values[0].value == ConstantValue::Integer(0)
            ),
            "{sql}"
        );
    }
}

#[test]
fn true_condition_returns_a_parser_error() {
    assert!(parse_select("SELECT throwIf(1, 'custom guard')").is_err());
    assert!(parse_select("SELECT throwIf(1)").is_err());
    assert!(parse_select("SELECT throwIf(0, 'ok', 3)").is_err());
}
