use crate::{ConstantValue, Projection, parse_select};

#[test]
fn preserves_order_delimiters_and_fragment_when_removing_keys() {
    for (query, expected) in [
        (
            "SELECT cutURLParameter('https://a.test/p?x=1&y=2&x=3#frag', 'x')",
            "https://a.test/p?y=2#frag",
        ),
        (
            "SELECT cutURLParameter('https://a.test/p?x=1&x=2', 'x')",
            "https://a.test/p",
        ),
        (
            "SELECT cutURLParameter('https://a.test/p?z=1#f', 'x')",
            "https://a.test/p?z=1#f",
        ),
        (
            "SELECT cutURLParameter('https://a.test/p?','x')",
            "https://a.test/p?",
        ),
    ] {
        assert!(
            matches!(
                parse_select(query).unwrap().projection,
                Projection::Constants(ref values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Text(expected.into())
            ),
            "{query}",
        );
    }
}
