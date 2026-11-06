use crate::{ConstantValue, Projection, parse_select};

#[test]
fn respects_integer_truthiness_null_conditions_and_common_types() {
    for (sql, expected) in [
        (
            "SELECT if(1, 'yes', 'no')",
            ConstantValue::Text("yes".into()),
        ),
        ("SELECT if(0, 3, 4)", ConstantValue::Integer(4)),
        (
            "SELECT if(NULL, 'yes', 'no')",
            ConstantValue::Text("no".into()),
        ),
        ("SELECT if(-3, NULL, 4)", ConstantValue::Null),
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
fn rejects_type_and_arity_errors() {
    for sql in [
        "SELECT if(1, 1)",
        "SELECT if(1, 2, 'text')",
        "SELECT if(1, 2, 3, 4)",
        "SELECT if('x', 1, 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
