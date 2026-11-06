use crate::{ConstantValue, Projection, parse_select};

#[test]
fn decodes_plus_and_percent_escaped_form_components() {
    for (sql, expected) in [
        (
            "SELECT decodeURLFormComponent('hello+world')",
            "hello world",
        ),
        ("SELECT decodeURLFormComponent('x%2By+z')", "x+y z"),
        ("SELECT decodeURLFormComponent('a%20b')", "a b"),
        ("SELECT decodeURLFormComponent('')", ""),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Text(expected.into())),
            "{sql}",
        );
    }
    for sql in [
        "SELECT decodeURLFormComponent('%GG')",
        "SELECT decodeURLFormComponent('%')",
        "SELECT decodeURLFormComponent('x', 'y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
