use rusthouse_query::parse_select;

#[test]
fn any_value_is_an_alias_for_any() {
    for (custom, canonical) in [
        ("SELECT ANY_VALUE(number) FROM numbers(4)", "SELECT ANY(number) FROM numbers(4)"),
        ("SELECT number, ANY_VALUE(number) FROM numbers(4) GROUP BY number",
         "SELECT number, ANY(number) FROM numbers(4) GROUP BY number"),
        ("SELECT ANY_VALUE(number), COUNT(*) FROM numbers(4)",
         "SELECT ANY(number), COUNT(*) FROM numbers(4)"),
    ] {
        assert_eq!(parse_select(custom).unwrap(), parse_select(canonical).unwrap());
    }
}
