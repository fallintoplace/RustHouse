use rusthouse_query::parse_select;

#[test]
fn grouped_count_having_alias_ignores_case() {
    let canonical = parse_select(
        "SELECT number, COUNT(*) AS Total FROM numbers(5) GROUP BY number HAVING Total > 0",
    )
    .unwrap();
    for alias in ["total", "TOTAL", "ToTaL"] {
        let query = parse_select(&format!(
            "SELECT number, COUNT(*) AS Total FROM numbers(5) GROUP BY number HAVING {alias} > 0"
        ))
        .unwrap();
        assert_eq!(query, canonical);
    }
    assert!(parse_select(
        "SELECT number, COUNT(*) AS Total FROM numbers(5) GROUP BY number HAVING Missing > 0"
    )
    .is_err());
}
