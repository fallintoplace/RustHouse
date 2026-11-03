use rusthouse_query::{Operator, Predicate, parse_select};

#[test]
fn negated_relational_operators_map_to_standard_comparisons() {
    for (symbol, expected) in [
        ("!<", Operator::Ge),
        ("!>", Operator::Le),
        ("!=", Operator::Ne),
    ] {
        let sql = format!("SELECT number FROM numbers(5) WHERE number {symbol} 2");
        let select = parse_select(&sql).unwrap();
        assert!(matches!(
            &select.predicates[0],
            Predicate::Comparison { operator, .. } if *operator == expected
        ), "{sql}");
    }
    assert!(parse_select("SELECT number FROM numbers(5) WHERE number ! 2").is_err());
}
