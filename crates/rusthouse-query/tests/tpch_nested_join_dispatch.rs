use rusthouse_query::parse_join;

#[test]
fn join_parser_ignores_joins_inside_derived_tables() {
    let sql = "SELECT c_count FROM (SELECT c_custkey, count(*) AS c_count FROM customer c LEFT JOIN orders o ON c.id = o.customer_id GROUP BY c_custkey) AS derived";
    assert!(parse_join(sql).unwrap().is_none());
}

#[test]
fn join_parser_ignores_joins_inside_correlated_subqueries() {
    let sql = "SELECT c_custkey FROM customer WHERE EXISTS (SELECT 1 FROM orders o JOIN lineitem l ON o.id = l.order_id WHERE o.id = c_custkey)";
    assert!(parse_join(sql).unwrap().is_none());
}

#[test]
fn actual_top_level_join_still_dispatches() {
    let sql = "SELECT c.id FROM customer c JOIN orders o ON c.id = o.customer_id";
    assert!(parse_join(sql).unwrap().is_some());
}
