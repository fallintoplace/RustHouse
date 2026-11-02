use rusthouse_query::{parse_select, Predicate};

#[test]
fn parses_canonical_tpch_q19_filter_without_exceeding_boolean_budget() {
    let sql = include_str!("../../../bench/tpch/queries/q19.sql");
    let predicates = sql.split_once("WHERE").unwrap().1.trim().trim_end_matches(';');
    let query = format!("SELECT count(*) FROM fused WHERE {predicates}");
    let select = parse_select(&query).unwrap();
    assert_eq!(select.or_predicates.len(), 2);
    for branch in std::iter::once(&select.predicates).chain(&select.or_predicates) {
        assert!(branch.iter().any(|predicate| matches!(predicate, Predicate::ColumnComparison { column, other, .. } if column == "p_partkey" && other == "l_partkey")));
        assert!(branch.len() >= 7);
    }
}
