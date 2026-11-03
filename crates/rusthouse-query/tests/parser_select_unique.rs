use rusthouse_query::parse_select;

#[test]
fn unique_select_is_equivalent_to_distinct() {
    for suffix in [
        "number FROM numbers(5)",
        "number FROM numbers(5) ORDER BY number DESC",
    ] {
        let unique = parse_select(&format!("SELECT UNIQUE {suffix}")).unwrap();
        let distinct = parse_select(&format!("SELECT DISTINCT {suffix}")).unwrap();
        assert_eq!(unique, distinct);
        assert!(unique.distinct);
    }
    assert!(parse_select("SELECT UNIQUE UNIQUE number FROM numbers(3)").is_err());
}
