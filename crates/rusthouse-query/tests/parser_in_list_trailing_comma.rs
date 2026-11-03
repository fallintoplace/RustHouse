use rusthouse_query::parse_select;

#[test]
fn in_and_not_in_lists_accept_final_comma_only() {
    for op in ["IN", "NOT IN"] {
        let canonical = parse_select(&format!(
            "SELECT number FROM numbers(8) WHERE number {op} (1, 3, 5)"
        ))
        .unwrap();
        let trailing = parse_select(&format!(
            "SELECT number FROM numbers(8) WHERE number {op} (1, 3, 5,)"
        ))
        .unwrap();
        assert_eq!(trailing, canonical);
        for values in [",", "1,,", "1,2,,"] {
            let sql = format!("SELECT number FROM numbers(8) WHERE number {op} ({values})");
            assert!(parse_select(&sql).is_err(), "{sql}");
        }
    }
}
