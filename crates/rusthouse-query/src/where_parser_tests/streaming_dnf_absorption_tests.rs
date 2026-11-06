use super::*;

fn equals(column: &str, number: &str) -> Predicate {
    Predicate::Comparison {
        column: column.into(),
        operator: Operator::Eq,
        value: number.into(),
    }
}

#[test]
fn identical_large_in_lists_do_not_need_cartesian_literal_clones() {
    let values = (0..4096).map(|n| n.to_string()).collect::<Vec<_>>();
    let heavy = Predicate::In {
        column: "number".into(),
        values,
    };
    let lhs = vec![vec![heavy.clone()]; 64];
    let rhs = vec![vec![heavy]; 64];
    let product = combine_and(lhs, rhs).unwrap();
    assert_eq!(product.len(), 1);
    assert_eq!(product[0].len(), 1);
}

#[test]
fn later_subsumption_removes_stronger_products_without_reordering() {
    let a = equals("a", "1");
    let b = equals("b", "2");
    let c = equals("c", "3");
    let d = equals("d", "4");
    let product = combine_and(
        vec![vec![a.clone(), b], vec![a.clone()]],
        vec![vec![c.clone()], vec![c.clone(), d]],
    )
    .unwrap();
    assert_eq!(product, vec![vec![a, c]]);
}

#[test]
fn unrelated_cross_products_still_enforce_final_branch_limit() {
    let lhs = (0..9).map(|n| vec![equals("a", &n.to_string())]).collect();
    let rhs = (0..9).map(|n| vec![equals("b", &n.to_string())]).collect();
    assert!(combine_and(lhs, rhs).is_err());
}
