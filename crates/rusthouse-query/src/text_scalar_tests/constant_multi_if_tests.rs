use crate::{ConstantValue, Projection, parse_select};

#[test]
fn picks_first_true_branch_and_treats_null_conditions_as_false() {
    for (sql, expected) in [
        (
            "SELECT multiIf(0, 'no', 1, 'yes', 'otherwise')",
            ConstantValue::Text("yes".into()),
        ),
        (
            "SELECT multiIf(NULL, 4, 1, 7, 9)",
            ConstantValue::Integer(7),
        ),
        ("SELECT multiIf(0, 4, 0, 7, 9)", ConstantValue::Integer(9)),
        ("SELECT multiIf(1, NULL, 2)", ConstantValue::Null),
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
fn rejects_invalid_arity_condition_types_and_branch_types() {
    for sql in [
        "SELECT multiIf(1, 2)",
        "SELECT multiIf(1, 2, 0, 3)",
        "SELECT multiIf('yes', 2, 3)",
        "SELECT multiIf(1, 'yes', 4)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
