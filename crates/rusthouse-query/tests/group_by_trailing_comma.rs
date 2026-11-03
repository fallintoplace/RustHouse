use rusthouse_query::parse_select;


#[test]
fn group_by_trailing_comma_matches_normal_grouping() {
    for (with_comma, without_comma) in [
        (
            "SELECT number, count(*) FROM numbers(6) GROUP BY number, ORDER BY number",
            "SELECT number, count(*) FROM numbers(6) GROUP BY number ORDER BY number",
        ),
        (
            "SELECT number, count(*) FROM numbers(6) GROUP BY number,;",
            "SELECT number, count(*) FROM numbers(6) GROUP BY number;",
        ),
    ] {
        assert_eq!(parse_select(with_comma).unwrap(), parse_select(without_comma).unwrap());
    }
    assert!(parse_select("SELECT number, count(*) FROM numbers(6) GROUP BY number,,").is_err());
}
