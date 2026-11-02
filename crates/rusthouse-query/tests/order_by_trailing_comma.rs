use rusthouse_query::parse_select;


#[test]
fn order_by_trailing_comma_preserves_sort_and_pagination() {
    for (with_comma, without_comma) in [
        (
            "SELECT number FROM numbers(6) ORDER BY number DESC, LIMIT 2",
            "SELECT number FROM numbers(6) ORDER BY number DESC LIMIT 2",
        ),
        (
            "SELECT number FROM numbers(6) ORDER BY number,;",
            "SELECT number FROM numbers(6) ORDER BY number;",
        ),
    ] {
        assert_eq!(parse_select(with_comma).unwrap(), parse_select(without_comma).unwrap());
    }
    assert!(parse_select("SELECT number FROM numbers(6) ORDER BY number,,").is_err());
}
