use super::*;

#[test]
fn one_eligible_generation_respects_exact_byte_budget() {
    let sizes = [(4, 100u64), (1, 25), (3, 10), (2, 5)];
    assert_eq!(
        retention_candidates_with_byte_budget(&sizes, 4, 3, &[], 24).unwrap(),
        (vec![], 0)
    );
    assert_eq!(
        retention_candidates_with_byte_budget(&sizes, 4, 3, &[], 25).unwrap(),
        (vec![1], 25)
    );
    assert_eq!(
        retention_candidates_with_byte_budget(&sizes, 4, 3, &[], u64::MAX).unwrap(),
        (vec![1], 25)
    );
    assert_eq!(
        retention_candidates_with_byte_budget(&sizes, 4, 3, &[1], u64::MAX).unwrap(),
        (vec![], 0)
    );
    assert!(retention_candidates_with_byte_budget(&[(1, 1), (1, 2)], 1, 1, &[], 100).is_err());
}
