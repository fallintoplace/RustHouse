use crate::{ConstantValue, Projection, parse_select};

#[test]
fn removes_named_query_parameters_and_preserves_fragments() {
    for (sql, expected) in [
        (
            "SELECT cutURLParameter('https://a/?x=1&y=2', 'x')",
            "https://a/?y=2",
        ),
        (
            "SELECT cutURLParameter('https://a/?x=1&x=2#frag', 'x')",
            "https://a/#frag",
        ),
        (
            "SELECT cutURLParameter('https://a/?x=1', 'missing')",
            "https://a/?x=1",
        ),
        (
            "SELECT cutURLParameter('https://a/path#frag', 'x')",
            "https://a/path#frag",
        ),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Text(expected.into())),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT cutURLParameter('x')").is_err());
}
