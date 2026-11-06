use crate::parse_select;

#[test]
fn identical_or_branches_and_and_predicates_share_one_plan() {
    let sql = "SELECT number FROM numbers(10) WHERE             (number = 1 AND number = 1) OR (number = 1) OR             (number = 2 AND number >= 0 AND number = 2)";
    let query = parse_select(sql).unwrap();
    assert_eq!(query.predicates.len(), 1);
    assert_eq!(query.or_predicates.len(), 1);
    assert_eq!(query.or_predicates[0].len(), 2);
}

#[test]
fn equivalent_conjunctions_deduplicate_regardless_of_order() {
    let query = parse_select(
        "SELECT number FROM numbers(10) WHERE              (number > 1 AND number < 7) OR (number < 7 AND number > 1)",
    ).unwrap();
    assert_eq!(query.predicates.len(), 2);
    assert!(query.or_predicates.is_empty());
}
