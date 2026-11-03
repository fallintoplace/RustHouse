use rusthouse_query::parse_select;

#[test]
fn single_source_alias_can_omit_as() {
    let bare = parse_select("SELECT number FROM numbers(4) n WHERE number > 1").unwrap();
    let explicit = parse_select("SELECT number FROM numbers(4) AS n WHERE number > 1").unwrap();
    assert_eq!(bare.source, explicit.source);
    assert_eq!(bare.predicates, explicit.predicates);

    let quoted = parse_select("SELECT number FROM numbers(4) `where` ORDER BY number").unwrap();
    assert!(quoted.order_by.is_some());
    assert!(parse_select("SELECT number FROM numbers(4) ORDER BY number").is_ok());
    assert!(parse_select("SELECT number FROM numbers(4) LIMIT 2").is_ok());
    assert!(parse_select("SELECT 1 FROM DUAL n").is_err());
}
