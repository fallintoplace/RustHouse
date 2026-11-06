use super::*;

#[test]
fn protects_active_latest_and_pinned_generations_under_budget() {
    let generations = [(5, 1u64), (1, 7), (2, 12), (3, 3), (4, 2)];
    assert_eq!(
        retention_candidates_with_byte_budget(&generations, 5, 1, &[3], 9).unwrap(),
        (vec![1, 4], 9)
    );
    assert_eq!(
        retention_candidates_with_byte_budget(&generations, 5, 1, &[3], 21).unwrap(),
        (vec![1, 2, 4], 21)
    );
    assert!(retention_candidates_with_byte_budget(&generations, 5, 1, &[99], 20).is_err());
}
