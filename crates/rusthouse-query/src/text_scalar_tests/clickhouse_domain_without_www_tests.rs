use crate::{ConstantValue, Projection, parse_select};

#[test]
fn extracts_domain_without_at_most_one_www_prefix() {
    for (sql, expected) in [
        (
            "SELECT domainWithoutWWW('https://www.example.org/p')",
            "example.org",
        ),
        (
            "SELECT domainWithoutWWW('www.example.org:80/p')",
            "example.org",
        ),
        (
            "SELECT domainWithoutWWW('https://www.www.example.org')",
            "www.example.org",
        ),
        (
            "SELECT domainWithoutWWW('https://example.org')",
            "example.org",
        ),
        ("SELECT domainWithoutWWW('not-a-url')", "not-a-url"),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Text(expected.into())),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT domainWithoutWWW('x', 'y')").is_err());
}
