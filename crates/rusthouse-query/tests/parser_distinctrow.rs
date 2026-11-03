use rusthouse_query::parse_select;

#[test]
fn distinctrow_matches_distinct() {
    let query = parse_select("SELECT DISTINCTROW number FROM numbers(4)").unwrap();
    assert!(query.distinct);
    assert!(parse_select("SELECT DISTINCTROW number FROM numbers(4) DISTINCT").is_err());
}
