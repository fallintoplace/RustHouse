use crate::parse_select;

#[test]
fn unsupported_subquery_has_specific_diagnostic() {
    let err = parse_select(
        "SELECT count(*) FROM orders WHERE o_orderkey IN (SELECT l_orderkey FROM lineitem)",
    )
    .unwrap_err();
    assert!(err.0.contains("IN (SELECT ...) and NOT IN"), "{err}");
}
