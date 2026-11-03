use rusthouse_query::parse_select;
#[test]
fn top_n_uses_limit_and_rejects_mixed_syntax() {
    assert_eq!(parse_select("SELECT TOP 2 number FROM numbers(5)").unwrap().limit, Some(2));
    assert_eq!(parse_select("SELECT TOP (3) number FROM numbers(5)").unwrap().limit, Some(3));
    assert!(parse_select("SELECT TOP 2 number FROM numbers(5) LIMIT 3").is_err());
    assert!(parse_select("SELECT TOP -1 number FROM numbers(5)").is_err());
}
