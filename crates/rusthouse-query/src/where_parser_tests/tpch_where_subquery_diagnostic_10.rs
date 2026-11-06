use crate::parse_select;

#[test]
fn unsupported_subquery_has_specific_diagnostic() {
    let err = parse_select("SELECT count(*) FROM partsupp WHERE ps_supplycost = (SELECT min(ps_supplycost) FROM partsupp)").unwrap_err();
    assert!(err.0.contains("scalar WHERE subqueries"), "{err}");
}
