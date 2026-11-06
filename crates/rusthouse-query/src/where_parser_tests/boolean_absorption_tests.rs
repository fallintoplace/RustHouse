use super::*;
use crate::{execute, parse_select};

fn rows(where_clause: &str) -> Vec<u8> {
    let sql = format!("SELECT number FROM numbers(20) WHERE {where_clause} ORDER BY number");
    let query = parse_select(&sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    output
}

#[test]
fn redundant_disjunctions_and_conjunctions_keep_exact_rows() {
    for base in ["number = 2", "number >= 3", "number % 2 = 0"] {
        assert_eq!(
            rows(base),
            rows(&format!("({base}) OR (({base}) AND number < 7)")),
        );
        assert_eq!(
            rows(base),
            rows(&format!("(({base}) AND number < 7) OR ({base})")),
        );
        assert_eq!(
            rows(&format!("NOT ({base})")),
            rows(&format!("NOT (({base}) OR (({base}) AND number < 7))")),
        );
    }
}

#[test]
fn repeated_factors_no_longer_explode_the_dnf_limit() {
    let groups = (0..7)
        .map(|_| "(number = 1 OR number = 2)")
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!("SELECT number FROM numbers(8) WHERE {groups} ORDER BY number");
    let select = parse_select(&sql).unwrap();
    assert!(select.or_predicates.len() <= 1);
    let mut output = Vec::new();
    execute(&select, None, &mut output).unwrap();
    assert_eq!(output, b"number\n1\n2\n");
}

#[test]
fn empty_and_distinct_branches_preserve_boolean_identities() {
    let eq_one = Predicate::Comparison {
        column: "number".into(),
        operator: Operator::Eq,
        value: "1".into(),
    };
    let eq_two = Predicate::Comparison {
        column: "number".into(),
        operator: Operator::Eq,
        value: "2".into(),
    };
    assert_eq!(
        unique_branches(vec![
            vec![eq_one.clone(), eq_two.clone()],
            vec![eq_one.clone()],
        ]),
        vec![vec![eq_one]]
    );
    assert_eq!(
        unique_branches(vec![vec![eq_two], vec![],]),
        vec![Vec::<Predicate>::new()]
    );
}
