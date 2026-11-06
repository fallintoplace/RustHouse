use crate::{ConstantValue, Projection, parse_select};

#[test]
fn removes_one_www_prefix_without_changing_other_url_components() {
    for (sql, expected) in [
        (
            "SELECT cutWWW('https://www.example.org/a?b=1')",
            "https://example.org/a?b=1",
        ),
        ("SELECT cutWWW('www.example.org/path')", "example.org/path"),
        (
            "SELECT cutWWW('https://www.www.example.org')",
            "https://www.example.org",
        ),
        (
            "SELECT cutWWW('https://example.org')",
            "https://example.org",
        ),
        (
            "SELECT cutWWW('https://u@www.example.org:80/x')",
            "https://u@example.org:80/x",
        ),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Text(expected.into())),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT cutWWW('a', 'b')").is_err());
}
