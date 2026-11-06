use super::*;

#[test]
fn fully_retained_generations_have_no_budgeted_work() {
    let generations = (1..=16_384_u64).map(|id| (id, 4)).collect::<Vec<_>>();
    assert_eq!(
        retention_candidates_with_byte_budget(
            &generations,
            16_384,
            generations.len(),
            &[],
            u64::MAX,
        )
        .unwrap(),
        (Vec::<u64>::new(), 0),
    );
    assert!(
        retention_candidates_with_byte_budget(&[(1, 4), (1, 8)], 1, 2, &[], u64::MAX,).is_err()
    );
}
