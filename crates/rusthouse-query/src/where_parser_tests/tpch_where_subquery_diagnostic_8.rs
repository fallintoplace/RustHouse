use crate::parse_select;

#[test]
fn unsupported_subquery_has_specific_diagnostic() {
    let err = parse_select("SELECT count(*) FROM orders WHERE EXISTS (SELECT * FROM lineitem WHERE l_orderkey = o_orderkey)").unwrap_err();
    assert!(err.0.contains("EXISTS and NOT EXISTS subqueries"), "{err}");
}
