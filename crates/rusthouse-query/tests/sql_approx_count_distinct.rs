use rusthouse_query::{execute, parse_select};

fn output(sql: &str) -> Vec<u8> {
    let query = parse_select(sql).unwrap();
    let mut buffer = Vec::new();
    execute(&query, None, &mut buffer).unwrap();
    buffer
}

#[test]
fn approximate_distinct_matches_uniq() {
    for rows in [0, 1, 5] {
        let standard = format!("SELECT APPROX_COUNT_DISTINCT(number) FROM numbers({rows})");
        let clickhouse = format!("SELECT uniq(number) FROM numbers({rows})");
        assert_eq!(parse_select(&standard).unwrap(), parse_select(&clickhouse).unwrap());
        assert_eq!(output(&standard), output(&clickhouse));
    }
}

#[test]
fn requires_exactly_one_column() {
    assert!(parse_select("SELECT approx_count_distinct() FROM numbers(3)").is_err());
    assert!(parse_select("SELECT approx_count_distinct(number, number) FROM numbers(3)").is_err());
}
